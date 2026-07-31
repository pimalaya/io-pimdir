//! [`PimdirStore`]: the std store that services [`io_replica`]'s storage seam.
//!
//! It persists a [`ReplicaHub`] per collection — one shared item plus a base per
//! source — and implements [`ReplicaStorage`] for one source: `load` projects
//! the hub for that source, `write` absorbs the source's writes back. A
//! single-source store is the N=1 case (one binding per item). Unlinked, freshly
//! probed placements have no link id to key an item on yet, so they are held
//! in-memory as a residual until a `Meta` upgrade resolves their link id.
//!
//! [`ReplicaStorage`]: io_replica::client::ReplicaStorage

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::sync::atomic::{AtomicU64, Ordering};
use std::{
    collections::BTreeMap,
    fmt, fs,
    io::{self, ErrorKind, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use io_replica::{
    change::ReplicaWriteOp,
    client::ReplicaStorage,
    collection::{ReplicaCheckpoint, ReplicaCollectionId},
    hub::{ReplicaHub, ReplicaHubConflict, ReplicaHubItem, ReplicaSourceBinding, ReplicaSourceId},
    object::ReplicaHash,
    placement::{ReplicaBase, ReplicaHandle, ReplicaLinkId, ReplicaMeta, ReplicaPlacement},
    storage::ReplicaLoaded,
};
use rusqlite::{Connection, OptionalExtension, Row, named_params, params};

use crate::{codec, sql};

/// A pimdir store opened as one source (`"left"`, `"right"`, `"phone"`, …). The
/// underlying database and blobs are shared; several sources of one store are
/// several handles over the same files.
pub struct PimdirStore {
    conn: Connection,
    blobs: PathBuf,
    source: ReplicaSourceId,
    /// Unlinked probed placements, awaiting the `Meta` upgrade that gives them a
    /// link id; kept in memory (empty at rest between syncs).
    residual: Vec<ReplicaPlacement>,
}

impl PimdirStore {
    /// Opens (creating if absent) the store rooted at `dir` as source `source`.
    pub fn open(dir: impl AsRef<Path>, source: impl Into<String>) -> Result<Self, PimdirError> {
        let dir = dir.as_ref();
        fs::create_dir_all(dir)?;
        let blobs = dir.join("objects");
        fs::create_dir_all(&blobs)?;

        let conn = Connection::open(dir.join("pimdir.db"))?;
        // NOTE: `busy_timeout` lets several source handles of one store (§7's
        // single-owner process opening `"left"` and `"right"` over the same
        // files) briefly wait out each other's write transaction instead of
        // failing with `SQLITE_BUSY`.
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;",
        )?;

        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version < sql::VERSION {
            conn.execute_batch(sql::MIGRATION_0001)?;
            conn.pragma_update(None, "user_version", sql::VERSION)?;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis().to_string())
                .unwrap_or_default();
            conn.execute(
                "INSERT OR IGNORE INTO store_meta(id, version, hash_algo, created_at) \
                 VALUES(1, ?1, ?2, ?3)",
                params![sql::VERSION, "blake3", now],
            )?;
        }

        Ok(Self {
            conn,
            blobs,
            source: ReplicaSourceId(source.into()),
            residual: Vec::new(),
        })
    }

    /// Loads a collection's full [`ReplicaHub`] — every source's items and
    /// bindings, not only this handle's source.
    ///
    /// [`load`](ReplicaStorage::load) projects the hub for one source; a
    /// multi-source consumer (a two-sided sync driving one handle per source
    /// over the shared files) reads the whole hub to project each side and to
    /// spot items held by a single source.
    pub fn load_hub(&self, collection: &str) -> Result<ReplicaHub, PimdirError> {
        Ok(load_hub(&self.conn, collection)?)
    }

    /// Declares a collection's media type (`kind`), creating the collection if
    /// absent and updating its kind otherwise.
    ///
    /// The kind is an [IANA media type](https://www.iana.org/assignments/media-types)
    /// (`message/rfc822`, `text/vcard`, `text/calendar`, …) — static consumer
    /// configuration, not something the sync engine derives — so a consumer
    /// sets it out of band from the [`ReplicaStorage`] seam. This is what makes
    /// the store self-describing (§4.3) and lets one store hold several item
    /// kinds. The lazy collection creation inside [`write`](ReplicaStorage::write)
    /// uses `ON CONFLICT DO NOTHING`, so it never clobbers a kind set here,
    /// whichever runs first.
    pub fn ensure_collection(&self, collection: &str, kind: &str) -> Result<(), PimdirError> {
        self.conn.execute(
            sql::SET_COLLECTION_KIND,
            named_params! { ":collection": collection, ":kind": kind },
        )?;
        Ok(())
    }

    /// The declared media type of a collection, or `None` if the store has
    /// never seen it. An empty string means the collection exists but was
    /// created lazily by a sync before any [`ensure_collection`](Self::ensure_collection)
    /// declared its kind.
    pub fn collection_kind(&self, collection: &str) -> Result<Option<String>, PimdirError> {
        Ok(self
            .conn
            .query_row(
                sql::LOAD_KIND,
                named_params! { ":collection": collection },
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    }
}

/// A read-only handle to a pimdir store's content-addressed blob directory,
/// independent of the SQLite [`Connection`].
///
/// A body can be read through it while the [`PimdirStore`] is mutably borrowed
/// to service a sync (e.g. a remote reads a stored body back to re-upload it as
/// a cross-source copy). Cheap to clone: it wraps only the `objects/` path.
#[derive(Clone, Debug)]
pub struct PimdirBlobs {
    root: PathBuf,
}

impl PimdirBlobs {
    /// Opens the blob reader for the store rooted at `dir`.
    pub fn open(dir: impl AsRef<Path>) -> Self {
        Self {
            root: dir.as_ref().join("objects"),
        }
    }

    /// Reads the body stored under `hash` from the sharded layout, or `None`
    /// when absent.
    pub fn get(&self, hash: &ReplicaHash) -> io::Result<Option<Vec<u8>>> {
        match fs::read(blob_path(&self.root, &hash.0)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// Opens a stored object as a readable stream, or `None` when absent — the
    /// append side of bounded-memory transfer, so a body is uploaded without
    /// being read whole into memory. The returned file's metadata gives the
    /// octet length a protocol that needs it up front (IMAP `APPEND`) requires.
    pub fn reader(&self, hash: &ReplicaHash) -> io::Result<Option<fs::File>> {
        match fs::File::open(blob_path(&self.root, &hash.0)) {
            Ok(file) => Ok(Some(file)),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// Opens a streaming writer for a new object: bytes are written to a
    /// temporary file and placed at their content-addressed path only on
    /// [`commit`](PimdirBlobWriter::commit), once the hash is known. The store
    /// is hash-agnostic, so the caller hashes the bytes as it writes them.
    pub fn writer(&self) -> io::Result<PimdirBlobWriter> {
        fs::create_dir_all(&self.root)?;
        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = self.root.join(format!(".tmp-{}-{seq}", std::process::id()));
        let file = fs::File::create(&tmp)?;
        Ok(PimdirBlobWriter {
            root: self.root.clone(),
            tmp,
            file: Some(file),
            written: 0,
        })
    }
}

/// A unique-per-write temp-file discriminator, so concurrent writers of one
/// store do not collide on the staging file.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// A streaming writer for one new blob (see [`PimdirBlobs::writer`]).
///
/// It is a [`Write`] sink over a temporary file; [`commit`](Self::commit) fsyncs
/// and renames it into the content-addressed path once the caller knows the
/// hash. Dropped without a commit (an error mid-stream), it removes the temp.
pub struct PimdirBlobWriter {
    root: PathBuf,
    tmp: PathBuf,
    file: Option<fs::File>,
    written: u64,
}

impl PimdirBlobWriter {
    /// Finalises the object under `hash`: fsync, then atomically rename the temp
    /// file into its sharded content-addressed path. A body already present
    /// (dedup) keeps the stored copy and drops the temp. Returns the object's
    /// byte size.
    pub fn commit(mut self, hash: &ReplicaHash) -> io::Result<u64> {
        let file = self.file.take().expect("writer open");
        file.sync_all()?;
        drop(file);

        let path = blob_path(&self.root, &hash.0);
        if path.exists() {
            let _ = fs::remove_file(&self.tmp);
            return Ok(self.written);
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&self.tmp, &path)?;
        Ok(self.written)
    }
}

impl Write for PimdirBlobWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let file = self.file.as_mut().expect("writer open");
        let n = file.write(buf)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.as_mut().expect("writer open").flush()
    }
}

impl Drop for PimdirBlobWriter {
    fn drop(&mut self) {
        // Uncommitted (an error mid-stream): best-effort remove the temp file.
        if self.file.is_some() {
            let _ = fs::remove_file(&self.tmp);
        }
    }
}

impl ReplicaStorage for PimdirStore {
    type Error = PimdirError;

    fn load(&self, collection: &ReplicaCollectionId) -> Result<ReplicaLoaded, Self::Error> {
        let hub = load_hub(&self.conn, &collection.0)?;
        let mut placements = hub.project(collection, &self.source);
        placements.extend(
            self.residual
                .iter()
                .filter(|p| &p.collection == collection)
                .cloned(),
        );

        let checkpoint = self
            .conn
            .query_row(
                sql::LOAD_CHECKPOINT,
                named_params! { ":collection": collection.0, ":source": self.source.0 },
                |r| r.get::<_, Option<Vec<u8>>>(0),
            )
            .optional()?
            .flatten()
            .map(ReplicaCheckpoint);

        Ok(ReplicaLoaded {
            placements,
            checkpoint,
        })
    }

    fn lookup_objects(
        &self,
        links: &[ReplicaLinkId],
    ) -> Result<BTreeMap<ReplicaLinkId, ReplicaHash>, Self::Error> {
        let ids: Vec<&str> = links.iter().map(|l| l.0.as_str()).collect();
        let json = serde_json::to_string(&ids)?;

        let mut map = BTreeMap::new();
        let mut stmt = self.conn.prepare(sql::LOOKUP_OBJECTS)?;
        let rows = stmt.query_map(named_params! { ":links": json }, |r| {
            Ok((
                ReplicaLinkId(r.get::<_, String>(0)?),
                ReplicaHash(r.get::<_, String>(1)?),
            ))
        })?;
        for row in rows {
            let (link, hash) = row?;
            map.insert(link, hash);
        }

        // NOTE: a body hydrated on a not-yet-linked residual placement.
        for placement in &self.residual {
            if let (Some(link), Some(object)) = (&placement.link_id, &placement.object) {
                if links.contains(link) {
                    map.entry(link.clone()).or_insert_with(|| object.clone());
                }
            }
        }

        Ok(map)
    }

    fn write(&mut self, ops: Vec<ReplicaWriteOp>) -> Result<(), Self::Error> {
        let blobs = self.blobs.clone();
        let source = self.source.clone();
        // Placement/drop ops routed to the hub, grouped by collection.
        let mut hub_ops: BTreeMap<String, Vec<ReplicaWriteOp>> = BTreeMap::new();

        let tx = self.conn.transaction()?;
        for op in ops {
            match op {
                ReplicaWriteOp::StoreObject { object, body } => {
                    // NOTE: a byteless op indexes an object the consumer already
                    // streamed into the blob store during a fetch (bounded-memory
                    // transfer); inline bytes are the buffered path.
                    if let Some(body) = body {
                        write_blob(&blobs, &object.hash.0, &body)?;
                    }
                    tx.execute(
                        sql::STORE_OBJECT,
                        named_params! { ":hash": object.hash.0, ":size": object.size as i64 },
                    )?;
                }
                ReplicaWriteOp::SetCheckpoint {
                    collection,
                    checkpoint,
                } => {
                    tx.execute(
                        sql::ENSURE_COLLECTION,
                        named_params! { ":collection": collection.0 },
                    )?;
                    tx.execute(
                        sql::UPSERT_CHECKPOINT,
                        named_params! {
                            ":collection": collection.0,
                            ":source": source.0,
                            ":checkpoint": checkpoint.0,
                        },
                    )?;
                }
                ReplicaWriteOp::UpsertPlacement(placement) => {
                    if placement.link_id.is_some() {
                        drop_residual(&mut self.residual, &placement.collection, &placement.handle);
                        hub_ops
                            .entry(placement.collection.0.clone())
                            .or_default()
                            .push(ReplicaWriteOp::UpsertPlacement(placement));
                    } else {
                        // NOTE: not yet linked — stage in the residual until a
                        // Meta upgrade resolves its link id.
                        match self.residual.iter().position(|r| {
                            r.collection == placement.collection && r.handle == placement.handle
                        }) {
                            Some(index) => self.residual[index] = placement,
                            None => self.residual.push(placement),
                        }
                    }
                }
                ReplicaWriteOp::DropPlacement { collection, handle } => {
                    drop_residual(&mut self.residual, &collection, &handle);
                    hub_ops
                        .entry(collection.0.clone())
                        .or_default()
                        .push(ReplicaWriteOp::DropPlacement { collection, handle });
                }
            }
        }

        // Absorb each collection's placement writes into its hub and save it.
        for (collection, ops) in hub_ops {
            let mut hub = load_hub(&tx, &collection)?;
            hub.absorb(&source, &ops);
            save_hub(&tx, &collection, &hub)?;
        }

        tx.execute(sql::RECOMPUTE_REFCOUNTS, [])?;
        let garbage: Vec<String> = {
            let mut stmt = tx.prepare(sql::LIST_GARBAGE_OBJECTS)?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            let mut hashes = Vec::new();
            for row in rows {
                hashes.push(row?);
            }
            hashes
        };
        tx.execute(sql::DELETE_GARBAGE_OBJECTS, [])?;
        tx.commit()?;

        for hash in garbage {
            remove_blob(&blobs, &hash)?;
        }
        Ok(())
    }
}

/// Removes any residual placement matching `(collection, handle)`.
fn drop_residual(
    residual: &mut Vec<ReplicaPlacement>,
    collection: &ReplicaCollectionId,
    handle: &ReplicaHandle,
) {
    residual.retain(|r| !(&r.collection == collection && &r.handle == handle));
}

/// Loads a collection's [`ReplicaHub`] (items + per-source bindings + policy).
fn load_hub(conn: &Connection, collection: &str) -> rusqlite::Result<ReplicaHub> {
    let mut hub = ReplicaHub::default();

    if let Some(policy) = conn
        .query_row(
            sql::LOAD_CONFLICT,
            named_params! { ":collection": collection },
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        hub.conflict = conflict_from_str(&policy);
    }

    let mut items = conn.prepare(sql::LOAD_ITEMS)?;
    let rows = items.query_map(named_params! { ":collection": collection }, item_from_row)?;
    for row in rows {
        let (link, item) = row?;
        hub.items.insert(link, item);
    }

    let mut bindings = conn.prepare(sql::LOAD_BINDINGS)?;
    let rows = bindings.query_map(
        named_params! { ":collection": collection },
        binding_from_row,
    )?;
    for row in rows {
        let (link, source, binding) = row?;
        if let Some(item) = hub.items.get_mut(&link) {
            item.sources.insert(source, binding);
        }
    }

    Ok(hub)
}

/// Replaces a collection's persisted hub with `hub` (delete-all then re-insert;
/// bindings cascade). Objects are indexed by the write batch's `StoreObject`s
/// and refcounted afterwards.
fn save_hub(conn: &Connection, collection: &str, hub: &ReplicaHub) -> rusqlite::Result<()> {
    conn.execute(
        sql::ENSURE_COLLECTION,
        named_params! { ":collection": collection },
    )?;
    conn.execute(
        sql::SET_CONFLICT,
        named_params! { ":collection": collection, ":conflict": conflict_to_str(hub.conflict) },
    )?;
    conn.execute(
        sql::DELETE_ITEMS,
        named_params! { ":collection": collection },
    )?;

    for (link, item) in &hub.items {
        conn.execute(
            sql::INSERT_ITEM,
            named_params! {
                ":collection": collection,
                ":link_id": link.0,
                ":flags": codec::flags_to_json(&item.flags),
                ":object_hash": item.object.as_ref().map(|o| o.0.as_str()),
                ":meta": item.meta.as_ref().map(|m| m.0.as_str()),
                ":level": codec::level_to_int(item.level),
                ":deleted": item.deleted as i64,
                ":conflicted": item.conflicted as i64,
                ":conflict_object": item.conflict_object.as_ref().map(|o| o.0.as_str()),
            },
        )?;

        for (source, binding) in &item.sources {
            let base_flags = binding
                .base
                .as_ref()
                .map(|b| codec::flags_to_json(&b.flags));
            conn.execute(
                sql::INSERT_BINDING,
                named_params! {
                    ":collection": collection,
                    ":link_id": link.0,
                    ":source": source.0,
                    ":handle": binding.handle.0,
                    ":base_flags": base_flags,
                    ":base_object": binding.base.as_ref().and_then(|b| b.object.as_ref()).map(|o| o.0.as_str()),
                    ":base_revision": binding.base.as_ref().and_then(|b| b.revision.as_deref()),
                },
            )?;
        }
    }

    Ok(())
}

fn item_from_row(row: &Row) -> rusqlite::Result<(ReplicaLinkId, ReplicaHubItem)> {
    let link: String = row.get(0)?;
    let flags: Option<String> = row.get(1)?;
    let object: Option<String> = row.get(2)?;
    let meta: Option<String> = row.get(3)?;
    let level: i64 = row.get(4)?;
    let deleted: i64 = row.get(5)?;
    let conflicted: i64 = row.get(6)?;
    let conflict_object: Option<String> = row.get(7)?;

    Ok((
        ReplicaLinkId(link),
        ReplicaHubItem {
            flags: codec::flags_from_json(flags.as_deref()),
            object: object.map(ReplicaHash),
            meta: meta.map(ReplicaMeta),
            level: codec::level_from_int(level),
            deleted: deleted != 0,
            conflicted: conflicted != 0,
            conflict_object: conflict_object.map(ReplicaHash),
            sources: BTreeMap::new(),
        },
    ))
}

fn binding_from_row(
    row: &Row,
) -> rusqlite::Result<(ReplicaLinkId, ReplicaSourceId, ReplicaSourceBinding)> {
    let link: String = row.get(0)?;
    let source: String = row.get(1)?;
    let handle: String = row.get(2)?;
    let base_flags: Option<String> = row.get(3)?;
    let base_object: Option<String> = row.get(4)?;
    let base_revision: Option<String> = row.get(5)?;

    let base = if base_flags.is_some() || base_object.is_some() || base_revision.is_some() {
        Some(ReplicaBase {
            flags: codec::flags_from_json(base_flags.as_deref()),
            revision: base_revision,
            object: base_object.map(ReplicaHash),
        })
    } else {
        None
    };

    Ok((
        ReplicaLinkId(link),
        ReplicaSourceId(source),
        ReplicaSourceBinding {
            handle: ReplicaHandle(handle),
            base,
        },
    ))
}

fn conflict_from_str(value: &str) -> ReplicaHubConflict {
    match value {
        "prefer-incoming" => ReplicaHubConflict::PreferIncoming,
        "prefer-existing" => ReplicaHubConflict::PreferExisting,
        _ => ReplicaHubConflict::Manual,
    }
}

fn conflict_to_str(policy: ReplicaHubConflict) -> &'static str {
    match policy {
        ReplicaHubConflict::Manual => "manual",
        ReplicaHubConflict::PreferIncoming => "prefer-incoming",
        ReplicaHubConflict::PreferExisting => "prefer-existing",
    }
}

/// The sharded on-disk path of a blob (`objects/<h[0:2]>/<h[2:4]>/<hash>`),
/// falling back to a flat path for hashes shorter than four characters.
fn blob_path(blobs: &Path, hash: &str) -> PathBuf {
    if hash.len() >= 4 {
        blobs.join(&hash[0..2]).join(&hash[2..4]).join(hash)
    } else {
        blobs.join(hash)
    }
}

/// Writes a blob atomically (temp → fsync → rename); a present hash is immutable
/// and left untouched.
fn write_blob(blobs: &Path, hash: &str, body: &[u8]) -> io::Result<()> {
    let path = blob_path(blobs, hash);
    if path.exists() {
        return Ok(());
    }
    let parent = path.parent().unwrap_or(blobs);
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(".{hash}.tmp"));
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(body)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, &path)
}

/// Removes a blob file; a missing file is not an error.
fn remove_blob(blobs: &Path, hash: &str) -> io::Result<()> {
    match fs::remove_file(blob_path(blobs, hash)) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Everything that can go wrong servicing the seam.
#[derive(Debug)]
pub enum PimdirError {
    Sql(rusqlite::Error),
    Io(io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for PimdirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PimdirError::Sql(err) => write!(f, "pimdir SQL error: {err}"),
            PimdirError::Io(err) => write!(f, "pimdir I/O error: {err}"),
            PimdirError::Json(err) => write!(f, "pimdir JSON error: {err}"),
        }
    }
}

impl std::error::Error for PimdirError {}

impl From<rusqlite::Error> for PimdirError {
    fn from(err: rusqlite::Error) -> Self {
        PimdirError::Sql(err)
    }
}

impl From<io::Error> for PimdirError {
    fn from(err: io::Error) -> Self {
        PimdirError::Io(err)
    }
}

impl From<serde_json::Error> for PimdirError {
    fn from(err: serde_json::Error) -> Self {
        PimdirError::Json(err)
    }
}
