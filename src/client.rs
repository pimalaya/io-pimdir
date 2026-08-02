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
    collections::{BTreeMap, HashMap},
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
    placement::{
        ReplicaBase, ReplicaFlags, ReplicaHandle, ReplicaLevel, ReplicaLinkId, ReplicaMeta,
        ReplicaPlacement,
    },
    storage::ReplicaLoaded,
};
use rusqlite::{
    Connection, ErrorCode, OptionalExtension, Row, TransactionBehavior, named_params, params,
};

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

/// A collection as seen by a client read (`list_collections`): its identity and
/// presentation, kind-agnostic. The sync bindings and per-source state are not
/// exposed here — a reader observes the shared truth only.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirCollection {
    /// The stable collection id (the mailbox name for a mail store).
    pub id: String,
    /// The declared IANA media type (`message/rfc822`, `text/vcard`, …), or the
    /// empty string when a sync created the collection before a kind was set.
    pub kind: String,
    /// The display name.
    pub name: String,
    /// The parent collection id, for a hierarchy.
    pub parent: Option<String>,
    /// A presentation colour hint.
    pub color: Option<String>,
    /// A free-text description.
    pub description: Option<String>,
    /// An explicit sort key; `None` sorts after the ordered ones.
    pub sort_order: Option<i64>,
}

/// One live item as seen by a client read (`list_items`/`get_item`): the shared
/// truth a domain projects (an envelope, a vCard, an event), kind-agnostic. The
/// `meta` is the raw stored summary — the reader parses it against its domain
/// schema. The `level` makes the read availability-aware: `level < Full` (and an
/// absent `object`) means the body is not local and a hydrate is needed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirItem {
    /// The message's public id (`items.seq`): a small, stable, store-global
    /// integer — the same across every mailbox the message is filed in — a
    /// consumer shows and passes back, instead of the long internal `link_id`.
    pub seq: i64,
    /// The cross-source link id (`Message-ID` for mail, UID for a vCard, …).
    /// Internal: a consumer keys reads and edits by `seq`, not this.
    pub link_id: ReplicaLinkId,
    /// The item's flag set.
    pub flags: ReplicaFlags,
    /// The raw per-domain summary blob, verbatim; `None` when never projected.
    pub meta: Option<ReplicaMeta>,
    /// The content-addressed body hash; `None` until a `Full` hydrate.
    pub object: Option<ReplicaHash>,
    /// The detail tier the item is hydrated to.
    pub level: ReplicaLevel,
}

impl PimdirStore {
    /// Opens (creating if absent) the store rooted at `dir` as source `source`.
    pub fn open(dir: impl AsRef<Path>, source: impl Into<String>) -> Result<Self, PimdirError> {
        let dir = dir.as_ref();
        fs::create_dir_all(dir)?;
        let blobs = dir.join("objects");
        fs::create_dir_all(&blobs)?;

        let conn = Connection::open(dir.join("pimdir.db"))?;
        // NOTE: `busy_timeout` lets several handles of one store wait out each
        // other's write transaction instead of failing with `SQLITE_BUSY` — §7's
        // single-owner process opening `"left"` and `"right"`, and a sync that
        // fans work across several same-source handles (one per worker) to overlap
        // network while the writes serialise. 30s absorbs a burst of large writes
        // (a first sync's per-mailbox meta insert) contending on the write lock.
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 30000;",
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

    /// Lists every collection in the store (client read surface).
    ///
    /// Ordered by `sort_order` then `id`, unordered collections last. This is a
    /// direct getter — it observes the shared truth and never mutates; writes go
    /// through io-replica's [`write`](ReplicaStorage::write) seam.
    pub fn list_collections(&self) -> Result<Vec<PimdirCollection>, PimdirError> {
        let mut stmt = self.conn.prepare(sql::LIST_COLLECTIONS)?;
        let rows = stmt.query_map([], |r| {
            Ok(PimdirCollection {
                id: r.get(0)?,
                kind: r.get(1)?,
                name: r.get(2)?,
                parent: r.get(3)?,
                color: r.get(4)?,
                description: r.get(5)?,
                sort_order: r.get(6)?,
            })
        })?;
        let mut collections = Vec::new();
        for row in rows {
            collections.push(row?);
        }
        Ok(collections)
    }

    /// A keyset page of a collection's live items (client read surface).
    ///
    /// `after` is the exclusive lower bound on `link_id` (`None` starts from the
    /// beginning); at most `limit` items are returned, ordered by `link_id`, so
    /// the last item's [`link_id`](PimdirItem::link_id) is the cursor for the
    /// next page. Tombstones (`deleted`) are excluded. Each item carries its
    /// `level`, so the caller sees a body's absence without probing the blobs.
    pub fn list_items(
        &self,
        collection: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<PimdirItem>, PimdirError> {
        let mut stmt = self.conn.prepare(sql::LIST_ITEMS_PAGE)?;
        let rows = stmt.query_map(
            named_params! {
                ":collection": collection,
                ":after": after.unwrap_or(""),
                ":limit": limit as i64,
            },
            read_item_from_row,
        )?;
        let mut items = Vec::new();
        for row in rows {
            items.push(row?);
        }
        Ok(items)
    }

    /// One live item by its public id `(collection, seq)`, or `None` (client read
    /// surface). A tombstoned item reads as `None`. The returned item carries its
    /// internal `link_id` for the caller to edit by.
    pub fn get_item(&self, collection: &str, seq: i64) -> Result<Option<PimdirItem>, PimdirError> {
        Ok(self
            .conn
            .query_row(
                sql::GET_ITEM,
                named_params! { ":collection": collection, ":seq": seq },
                read_item_from_row,
            )
            .optional()?)
    }

    /// Resolves an item's public id (`seq`) from its internal `link_id` — the
    /// inverse of [`get_item`](Self::get_item), for a consumer that just staged an
    /// add and wants the id the item now shows under.
    pub fn seq_for_link(
        &self,
        collection: &str,
        link_id: &str,
    ) -> Result<Option<i64>, PimdirError> {
        Ok(self
            .conn
            .query_row(
                sql::SEQ_BY_LINK,
                named_params! { ":collection": collection, ":link_id": link_id },
                |row| row.get(0),
            )
            .optional()?)
    }

    /// The distinct source names the store has synced against (across all
    /// collections). A client uses this to attribute its writes: a store synced
    /// as a single source (the local-sync case) has exactly one, so the app
    /// writes as it without configuration.
    pub fn distinct_sources(&self) -> Result<Vec<String>, PimdirError> {
        let mut stmt = self.conn.prepare(sql::LIST_SOURCES)?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut sources = Vec::new();
        for row in rows {
            sources.push(row?);
        }
        Ok(sources)
    }

    /// A collection's live (non-tombstone) item count (client read surface).
    pub fn count_items(&self, collection: &str) -> Result<u64, PimdirError> {
        let count: i64 = self.conn.query_row(
            sql::COUNT_ITEMS,
            named_params! { ":collection": collection },
            |r| r.get(0),
        )?;
        Ok(count.max(0) as u64)
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

        // BEGIN IMMEDIATE takes the single writer lock up front (§7): under WAL
        // reads never block, but two writers serialise here, and a writer that
        // cannot get the lock within `busy_timeout` fails fast and loud (`Busy`)
        // rather than deep inside the batch on a deferred lock upgrade.
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
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

        // Absorb each collection's placement writes into its hub, then persist
        // only what changed: diff the loaded hub against the absorbed one (touch
        // just the changed items/bindings), and adjust object refcounts by only
        // the per-hash change in references — never a whole-collection rewrite or
        // a global refcount recompute (both were O(N²) at scale).
        for (collection, ops) in hub_ops {
            let old_hub = load_hub(&tx, &collection)?;
            let mut new_hub = old_hub.clone();
            new_hub.absorb(&source, &ops);
            save_hub_diff(&tx, &collection, &old_hub, &new_hub)?;
            adjust_refcounts(&tx, &object_refs(&old_hub), &object_refs(&new_hub))?;
        }

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
        tx.commit().map_err(busy_or_sql)?;

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

/// Persists the change from `old` to `new` for a collection's hub by diffing the
/// two in memory and issuing only the item/binding inserts, updates and deletes
/// that actually differ — never a whole-collection delete-and-reinsert. So a
/// write touches O(changed rows), not O(collection size). Item deletes cascade to
/// their bindings (`PRAGMA foreign_keys = ON`).
fn save_hub_diff(
    conn: &Connection,
    collection: &str,
    old: &ReplicaHub,
    new: &ReplicaHub,
) -> rusqlite::Result<()> {
    conn.execute(
        sql::ENSURE_COLLECTION,
        named_params! { ":collection": collection },
    )?;
    if old.conflict != new.conflict {
        conn.execute(
            sql::SET_CONFLICT,
            named_params! { ":collection": collection, ":conflict": conflict_to_str(new.conflict) },
        )?;
    }

    // Items gone in `new`: delete (bindings cascade).
    for link in old.items.keys() {
        if !new.items.contains_key(link) {
            conn.execute(
                sql::DELETE_ITEM,
                named_params! { ":collection": collection, ":link_id": link.0 },
            )?;
        }
    }

    // Items added or changed in `new`.
    for (link, item) in &new.items {
        match old.items.get(link) {
            None => insert_item(conn, collection, link, item)?,
            Some(prev) => {
                if !item_columns_eq(prev, item) {
                    update_item(conn, collection, link, item)?;
                }
                save_bindings_diff(conn, collection, link, prev, item)?;
            }
        }
    }

    Ok(())
}

/// Whether two items' persisted columns (everything but their bindings) match.
fn item_columns_eq(a: &ReplicaHubItem, b: &ReplicaHubItem) -> bool {
    a.flags == b.flags
        && a.object == b.object
        && a.meta == b.meta
        && a.level == b.level
        && a.deleted == b.deleted
        && a.conflicted == b.conflicted
        && a.conflict_object == b.conflict_object
}

fn insert_item(
    conn: &Connection,
    collection: &str,
    link: &ReplicaLinkId,
    item: &ReplicaHubItem,
) -> rusqlite::Result<()> {
    // The public id is a property of the message: if this link id already has a
    // seq in any collection (the message is filed in another mailbox too), reuse
    // it, so all its placements share one id; otherwise draw a fresh store-global
    // id (never reused). A consumer keys on this small integer, not the link id.
    let seq: i64 = match conn
        .query_row(
            sql::SEQ_FOR_LINK_ANY,
            named_params! { ":link_id": link.0 },
            |row| row.get(0),
        )
        .optional()?
    {
        Some(existing) => existing,
        None => conn.query_row(sql::BUMP_NEXT_SEQ, [], |row| row.get(0))?,
    };
    conn.execute(
        sql::INSERT_ITEM,
        named_params! {
            ":collection": collection,
            ":link_id": link.0,
            ":seq": seq,
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
        insert_binding(conn, collection, link, source, binding)?;
    }
    Ok(())
}

fn update_item(
    conn: &Connection,
    collection: &str,
    link: &ReplicaLinkId,
    item: &ReplicaHubItem,
) -> rusqlite::Result<()> {
    conn.execute(
        sql::UPDATE_ITEM,
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
    Ok(())
}

/// Diffs one item's per-source bindings between `old` and `new`, issuing only the
/// binding inserts/updates/deletes that changed.
fn save_bindings_diff(
    conn: &Connection,
    collection: &str,
    link: &ReplicaLinkId,
    old: &ReplicaHubItem,
    new: &ReplicaHubItem,
) -> rusqlite::Result<()> {
    for source in old.sources.keys() {
        if !new.sources.contains_key(source) {
            conn.execute(
                sql::DELETE_BINDING,
                named_params! { ":collection": collection, ":link_id": link.0, ":source": source.0 },
            )?;
        }
    }
    for (source, binding) in &new.sources {
        match old.sources.get(source) {
            None => insert_binding(conn, collection, link, source, binding)?,
            Some(prev) if prev != binding => {
                update_binding(conn, collection, link, source, binding)?
            }
            Some(_) => {}
        }
    }
    Ok(())
}

fn insert_binding(
    conn: &Connection,
    collection: &str,
    link: &ReplicaLinkId,
    source: &ReplicaSourceId,
    binding: &ReplicaSourceBinding,
) -> rusqlite::Result<()> {
    conn.execute(
        sql::INSERT_BINDING,
        named_params! {
            ":collection": collection,
            ":link_id": link.0,
            ":source": source.0,
            ":handle": binding.handle.0,
            ":base_flags": binding.base.as_ref().map(|b| codec::flags_to_json(&b.flags)),
            ":base_object": binding.base.as_ref().and_then(|b| b.object.as_ref()).map(|o| o.0.as_str()),
            ":base_revision": binding.base.as_ref().and_then(|b| b.revision.as_deref()),
        },
    )?;
    Ok(())
}

fn update_binding(
    conn: &Connection,
    collection: &str,
    link: &ReplicaLinkId,
    source: &ReplicaSourceId,
    binding: &ReplicaSourceBinding,
) -> rusqlite::Result<()> {
    conn.execute(
        sql::UPDATE_BINDING,
        named_params! {
            ":collection": collection,
            ":link_id": link.0,
            ":source": source.0,
            ":handle": binding.handle.0,
            ":base_flags": binding.base.as_ref().map(|b| codec::flags_to_json(&b.flags)),
            ":base_object": binding.base.as_ref().and_then(|b| b.object.as_ref()).map(|o| o.0.as_str()),
            ":base_revision": binding.base.as_ref().and_then(|b| b.revision.as_deref()),
        },
    )?;
    Ok(())
}

/// The multiset of object references a hub holds — every item's `object` and
/// `conflict_object` plus every binding's `base.object` — keyed by hash. This is
/// exactly what the old global recompute counted, computed in memory so refcount
/// maintenance is a per-hash delta rather than a full-table rescan.
fn object_refs(hub: &ReplicaHub) -> HashMap<String, i64> {
    let mut refs: HashMap<String, i64> = HashMap::new();
    let mut bump = |hash: &ReplicaHash| *refs.entry(hash.0.clone()).or_insert(0) += 1;
    for item in hub.items.values() {
        if let Some(object) = &item.object {
            bump(object);
        }
        if let Some(conflict) = &item.conflict_object {
            bump(conflict);
        }
        for binding in item.sources.values() {
            if let Some(object) = binding.base.as_ref().and_then(|b| b.object.as_ref()) {
                bump(object);
            }
        }
    }
    refs
}

/// Applies the change in object references between two reference multisets as
/// per-hash refcount deltas (`refcount += new - old`), touching only hashes whose
/// count moved. A hash referenced by other collections keeps their share: the
/// delta reflects this collection's change alone.
fn adjust_refcounts(
    conn: &Connection,
    old: &HashMap<String, i64>,
    new: &HashMap<String, i64>,
) -> rusqlite::Result<()> {
    for (hash, new_count) in new {
        let delta = new_count - old.get(hash).copied().unwrap_or(0);
        if delta != 0 {
            conn.execute(
                sql::ADJUST_REFCOUNT,
                named_params! { ":delta": delta, ":hash": hash },
            )?;
        }
    }
    for (hash, old_count) in old {
        if !new.contains_key(hash) {
            conn.execute(
                sql::ADJUST_REFCOUNT,
                named_params! { ":delta": -old_count, ":hash": hash },
            )?;
        }
    }
    Ok(())
}

/// Maps a client-read row (`seq, link_id, flags, object_hash, meta, level`) to a
/// [`PimdirItem`]. Shared by `list_items` and `get_item`.
fn read_item_from_row(row: &Row) -> rusqlite::Result<PimdirItem> {
    let seq: i64 = row.get(0)?;
    let link: String = row.get(1)?;
    let flags: Option<String> = row.get(2)?;
    let object: Option<String> = row.get(3)?;
    let meta: Option<String> = row.get(4)?;
    let level: i64 = row.get(5)?;

    Ok(PimdirItem {
        seq,
        link_id: ReplicaLinkId(link),
        flags: codec::flags_from_json(flags.as_deref()),
        meta: meta.map(ReplicaMeta),
        object: object.map(ReplicaHash),
        level: codec::level_from_int(level),
    })
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
    /// Another writer holds the store's single write lock (§7); the caller
    /// should retry once the other writer (a sync, another client) is done.
    Busy,
}

impl fmt::Display for PimdirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PimdirError::Sql(err) => write!(f, "pimdir SQL error: {err}"),
            PimdirError::Io(err) => write!(f, "pimdir I/O error: {err}"),
            PimdirError::Json(err) => write!(f, "pimdir JSON error: {err}"),
            PimdirError::Busy => write!(
                f,
                "pimdir store is busy: another writer holds the write lock; retry once it releases"
            ),
        }
    }
}

/// Maps a SQLite busy/locked failure to the clear [`PimdirError::Busy`], leaving
/// any other error as a plain SQL error.
fn busy_or_sql(err: rusqlite::Error) -> PimdirError {
    match &err {
        rusqlite::Error::SqliteFailure(e, _)
            if matches!(e.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) =>
        {
            PimdirError::Busy
        }
        _ => PimdirError::Sql(err),
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
