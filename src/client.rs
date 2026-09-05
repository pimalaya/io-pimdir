//! # Client
//!
//! The std store: the three roles the format defines (STORAGE §8), one
//! handle each. [`PimdirStore`] owns the store and carries every write
//! that consults no source: retention, the collector, the queue rows a
//! cancellation removes. [`PimdirSourceStore`], which
//! [`for_source`](PimdirStore::for_source) yields, is the store as one
//! source: the load and write seam, the runner of the five verbs
//! against a [`PimdirRemote`], and the queue drain. [`PimdirProducer`]
//! enqueues and nothing else, [`PimdirReader`] reads and takes no lock.
//!
//! [`PimdirRemote`]: crate::remote::PimdirRemote
//! [`PimdirProducer`]: crate::client::producer::PimdirProducer

use alloc::{string::String, vec::Vec};

use std::{
    fmt, fs, io,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::Arc,
};

use rusqlite::{
    Connection, ErrorCode, OptionalExtension, Params, Row, TransactionBehavior, named_params,
};

use crate::{
    client::{lock::PimdirLock, reader::PimdirReader},
    codec::{self, PimdirActionError},
    hash::PimdirHashAlgo,
    hub::{PimdirHub, PimdirHubConflict, PimdirSourceId},
    sql,
};

pub mod blobs;
pub mod diagnostics;
pub mod producer;
pub mod reader;

mod lock;
mod run;
#[doc(inline)]
pub use run::PimdirRunError;
mod schema;
mod write;

/// A pimdir store held as its owner (STORAGE §8).
///
/// The write surface over the read surface every role shares.
pub struct PimdirStore {
    reader: PimdirReader,
    /// The exclusive owner lock (§8), held for the handle's lifetime and
    /// shared by every handle of this process, carrying the lock its
    /// writers and collector serialise on.
    pub(crate) lock: Arc<PimdirLock>,
    /// The account every collection this handle creates belongs to (§9.2).
    account: Option<String>,
}

impl Deref for PimdirStore {
    type Target = PimdirReader;

    fn deref(&self) -> &Self::Target {
        &self.reader
    }
}

impl DerefMut for PimdirStore {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.reader
    }
}

/// A pimdir store acting as one source: the sync seam (STORAGE §14).
///
/// Every operation means "as this side". Dereferences to the
/// [`PimdirStore`] it was made from.
pub struct PimdirSourceStore {
    store: PimdirStore,
    source: PimdirSourceId,
}

impl Deref for PimdirSourceStore {
    type Target = PimdirStore;

    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

impl DerefMut for PimdirSourceStore {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.store
    }
}

/// What a purge retired: rows, never bytes, which the collector frees.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PimdirPurgeReport {
    /// Retained items deleted.
    pub items: usize,
}

/// What a collection reclaimed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PimdirGcReport {
    /// Object rows dropped: bodies nothing references any more.
    pub objects: usize,
    /// Blob files unlinked: those rows' bodies and the orphans a crash left.
    pub blobs: usize,
    /// The bytes those files freed.
    pub bytes: u64,
}

impl PimdirStore {
    /// Opens (creating if absent) the store rooted at `dir` as its owner.
    ///
    /// Takes the store's exclusive advisory lock (§8) and holds it until
    /// the handle drops; a store another process owns is
    /// [`PimdirError::Owned`] at once, never a wait. A fresh database is
    /// created by running the migrations (§6); one above the current
    /// version, or from an earlier draft, is refused and recreated by its
    /// owner.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, PimdirError> {
        Self::open_with_hash(dir, None)
    }

    /// Opens the store rooted at `dir`, declaring the hash its objects
    /// are named by (§5) when it creates one; an existing store whose
    /// algorithm differs is [`PimdirError::HashAlgo`].
    pub fn open_with_hash(
        dir: impl AsRef<Path>,
        hash: Option<PimdirHashAlgo>,
    ) -> Result<Self, PimdirError> {
        let dir = dir.as_ref();
        fs::create_dir_all(dir)?;
        fs::create_dir_all(dir.join("objects"))?;

        let lock = PimdirLock::own(dir)?;

        let mut conn = Connection::open(dir.join("pimdir.db"))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 30000;",
        )?;
        schema::init(&mut conn, hash.unwrap_or_default())?;
        let hash = schema::hash_algo(&conn, hash)?;

        Ok(Self {
            reader: PimdirReader::over(conn, dir.to_path_buf(), hash),
            lock,
            account: None,
        })
    }

    /// Binds this handle to an account, so every collection it creates
    /// is grouped under it (§9.2).
    pub fn for_account(mut self, account: impl Into<String>) -> Self {
        self.account = Some(account.into());
        self
    }

    /// The account this handle writes under, `None` in a single-account store.
    pub fn account(&self) -> Option<&str> {
        self.account.as_deref()
    }

    /// Binds this handle to a source, yielding the sync seam.
    pub fn for_source(self, source: impl Into<String>) -> PimdirSourceStore {
        PimdirSourceStore {
            store: self,
            source: PimdirSourceId(source.into()),
        }
    }

    /// A collection's whole hub: every source's items and bindings.
    pub fn load_hub(&self, collection: impl AsRef<str>) -> Result<PimdirHub, PimdirError> {
        write::read_hub(&self.conn, collection.as_ref(), None)
    }

    /// Declares a collection's media type, creating the row if absent
    /// (§14). The lazy creation inside a write never overwrites it.
    pub fn ensure_collection(
        &self,
        collection: impl AsRef<str>,
        kind: &str,
    ) -> Result<(), PimdirError> {
        self.conn
            .execute(
                sql::SET_COLLECTION_KIND,
                named_params! {
                    ":collection": collection.as_ref(),
                    ":account": self.account.as_deref(),
                    ":kind": kind,
                },
            )
            .map_err(busy_or_sql)?;
        Ok(())
    }

    /// Regroups a collection under `account`, or out of one with `None` (§9.2).
    pub fn set_collection_account(
        &self,
        collection: impl AsRef<str>,
        account: Option<&str>,
    ) -> Result<(), PimdirError> {
        self.conn
            .execute(
                sql::SET_COLLECTION_ACCOUNT,
                named_params! { ":collection": collection.as_ref(), ":account": account },
            )
            .map_err(busy_or_sql)?;
        Ok(())
    }

    /// Sets a collection's cross-source conflict policy (SYNC §9).
    pub fn set_collection_conflict(
        &self,
        collection: impl AsRef<str>,
        policy: PimdirHubConflict,
    ) -> Result<(), PimdirError> {
        self.conn
            .execute(
                sql::SET_CONFLICT,
                named_params! {
                    ":collection": collection.as_ref(),
                    ":conflict": codec::conflict_to_str(policy),
                },
            )
            .map_err(busy_or_sql)?;
        Ok(())
    }

    /// Restates one item's ordering key (§9.3), outside the write path.
    pub fn set_sort_key(
        &self,
        collection: impl AsRef<str>,
        link_id: &str,
        sort_key: &str,
    ) -> Result<(), PimdirError> {
        self.conn
            .execute(
                sql::SET_SORT_KEY,
                named_params! {
                    ":collection": collection.as_ref(),
                    ":link_id": link_id,
                    ":sort_key": sort_key,
                },
            )
            .map_err(busy_or_sql)?;
        Ok(())
    }

    /// Gives a collection a new id, its contents following through the
    /// cascades (§14): the only safe way to change one.
    pub fn rename_collection(
        &self,
        collection: impl AsRef<str>,
        new_id: &str,
    ) -> Result<(), PimdirError> {
        self.conn
            .execute(
                sql::RENAME_COLLECTION,
                named_params! { ":collection": collection.as_ref(), ":new_id": new_id },
            )
            .map_err(busy_or_sql)?;
        Ok(())
    }
}

/// Retention (§11): the trash a store keeps, and the only deletes.
impl PimdirStore {
    /// Purges one retained item by its public id, reporting whether there
    /// was one; a live item is never reached.
    pub fn purge(&mut self, collection: impl AsRef<str>, seq: i64) -> Result<bool, PimdirError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;

        let pinned: Option<(Option<String>, Option<String>)> = tx
            .prepare(sql::PURGE_ITEM)?
            .query_row(
                named_params! { ":collection": collection.as_ref(), ":seq": seq },
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((object, conflict_object)) = pinned else {
            return Ok(false);
        };

        release_pins(&tx, [object, conflict_object].into_iter().flatten())?;
        tx.commit().map_err(busy_or_sql)?;
        Ok(true)
    }

    /// Purges every item retired strictly before `cutoff` (RFC 3339),
    /// store-wide: the cutoff is the caller's policy, never the store's clock.
    pub fn purge_retained_before(
        &mut self,
        cutoff: &str,
    ) -> Result<PimdirPurgeReport, PimdirError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;

        let pinned: Vec<(Option<String>, Option<String>)> = rows(
            &tx,
            sql::PURGE_RETAINED_BEFORE,
            named_params! { ":cutoff": cutoff },
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let items = pinned.len();
        release_pins(
            &tx,
            pinned
                .into_iter()
                .flat_map(|(object, conflict)| [object, conflict])
                .flatten(),
        )?;
        tx.commit().map_err(busy_or_sql)?;

        Ok(PimdirPurgeReport { items })
    }
}

/// Reclamation and repair (§5, §7): what no write does to the store.
impl PimdirStore {
    /// Reclaims what nothing references: the object rows at refcount
    /// zero, their bodies, and the orphan blobs a crash left.
    ///
    /// Takes the staging lock exclusively on an owning handle, and the
    /// process's own writer lock exclusively across the rows and the
    /// walk, so no writer is between a body and the row that pins it
    /// (§8). The rows go inside a transaction and the files after it.
    pub fn collect_garbage(&mut self) -> Result<PimdirGcReport, PimdirError> {
        let _staging = PimdirLock::collect(&self.dir)?;
        let lock = Arc::clone(&self.lock);
        let _collecting = lock.collecting();

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        let objects = tx.execute(sql::DELETE_GARBAGE_OBJECTS, [])?;
        tx.commit().map_err(busy_or_sql)?;

        let mut report = PimdirGcReport {
            objects,
            ..Default::default()
        };
        let mut exists = self.conn.prepare(sql::OBJECT_EXISTS)?;
        for blob in self.blobs().files()? {
            if exists.exists(named_params! { ":hash": blob.hash })? {
                continue;
            }
            fs::remove_file(&blob.path)?;
            report.blobs += 1;
            report.bytes += blob.size;
        }
        drop(exists);

        Ok(report)
    }

    /// Recomputes every refcount from the five pointer columns (§7),
    /// returning how many rows disagreed.
    pub fn recompute_refcounts(&self) -> Result<usize, PimdirError> {
        self.conn
            .execute(sql::RECOMPUTE_REFCOUNTS, [])
            .map_err(busy_or_sql)
    }

    /// Deletes the bindings whose item is gone, returning how many: the
    /// one dangling row a repair clears without guessing.
    pub fn clear_dangling_bindings(&self) -> Result<usize, PimdirError> {
        self.conn
            .execute(sql::DELETE_DANGLING_BINDINGS, [])
            .map_err(busy_or_sql)
    }
}

/// The queue's owner side (§15): cancelling and recording failures.
impl PimdirStore {
    /// Cancels one queue row as the store's owner, holding the role for
    /// the length of the call (§15.5); the store must exist already.
    pub fn cancel_action(dir: impl AsRef<Path>, id: i64) -> Result<bool, PimdirError> {
        let dir = dir.as_ref();
        if !dir.join("pimdir.db").is_file() {
            return Err(PimdirError::Uncreated);
        }

        Self::open(dir)?.drop_action(id)
    }

    /// Removes one queue row by request, pending or parked, releasing its
    /// body pin in the same transaction; reports whether it existed.
    pub fn drop_action(&mut self, id: i64) -> Result<bool, PimdirError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;

        let hash: Option<Option<String>> = tx
            .query_row(sql::CANCEL_ACTION, named_params! { ":id": id }, |r| {
                r.get(0)
            })
            .optional()?;
        let Some(hash) = hash else {
            return Ok(false);
        };

        release_pins(&tx, hash.into_iter())?;
        tx.commit().map_err(busy_or_sql)?;
        Ok(true)
    }

    /// Records a failed apply (§15.2): `None` bumps the attempts and
    /// leaves the row pending, `Some(error)` parks it, the attempt counted.
    pub fn fail_action(&self, id: i64, error: Option<&str>) -> Result<(), PimdirError> {
        let Some(error) = error else {
            self.conn
                .execute(sql::BUMP_ATTEMPTS, named_params! { ":id": id })
                .map_err(busy_or_sql)?;
            return Ok(());
        };

        self.conn
            .execute(
                sql::PARK_ACTION,
                named_params! { ":id": id, ":error": error },
            )
            .map_err(busy_or_sql)?;
        Ok(())
    }
}

impl PimdirSourceStore {
    /// The source this handle acts as.
    pub fn source(&self) -> &str {
        &self.source.0
    }

    /// Binds this handle to an account (§9.2), in either order with the source.
    pub fn for_account(mut self, account: impl Into<String>) -> Self {
        self.store = self.store.for_account(account);
        self
    }
}

/// Runs one statement and collects every row through `map`.
pub(crate) fn rows<T>(
    conn: &Connection,
    sql: &str,
    params: impl Params,
    map: impl FnMut(&Row) -> rusqlite::Result<T>,
) -> rusqlite::Result<Vec<T>> {
    conn.prepare(sql)?.query_map(params, map)?.collect()
}

/// Releases the pins a deleted row held, set-based, in the caller's
/// transaction.
pub(crate) fn release_pins(
    conn: &Connection,
    hashes: impl Iterator<Item = String>,
) -> Result<(), PimdirError> {
    let hashes: Vec<String> = hashes.collect();
    if hashes.is_empty() {
        return Ok(());
    }
    conn.execute(
        sql::RELEASE_PINS,
        named_params! { ":hashes": serde_json::to_string(&hashes)? },
    )?;
    Ok(())
}

/// Maps a SQLite busy or locked failure to [`PimdirError::Busy`].
pub(crate) fn busy_or_sql(err: rusqlite::Error) -> PimdirError {
    match &err {
        rusqlite::Error::SqliteFailure(e, _)
            if matches!(e.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) =>
        {
            PimdirError::Busy
        }
        _ => PimdirError::Sql(err),
    }
}

/// Everything that can go wrong servicing the store.
#[derive(Debug)]
pub enum PimdirError {
    /// The SQLite database refused a statement, or the connection failed.
    Sql(rusqlite::Error),
    /// The blob directory refused a read, a write or a rename.
    Io(io::Error),
    /// JSON encoding failed at the seam.
    Json(serde_json::Error),
    /// A queue action payload is malformed (§15.3).
    Action(PimdirActionError),
    /// A write resolved an existing binding to a different handle and was
    /// refused (§10): a binding pins one handle, and the one licensed
    /// rebind is a `Superseded` or `Rekeyed` drop in the same batch.
    Rebind {
        /// The collection holding the binding.
        collection: String,
        /// The identity it is keyed by.
        link_id: String,
        /// The source whose binding it is.
        source: String,
        /// The handle the binding holds, and keeps.
        bound: String,
        /// The handle the refused write carried.
        incoming: String,
    },
    /// The store's schema version is not one this crate services.
    Version {
        /// The store's `user_version`.
        found: i64,
    },
    /// The store has no schema yet; only the owner's open creates one.
    Uncreated,
    /// The two schema stamps disagree, which §4.2 defines as corruption.
    VersionMismatch {
        /// The store's `PRAGMA user_version`.
        user_version: i64,
        /// The version its `store_meta` row records.
        store_meta: i64,
    },
    /// The store was created by an earlier draft and lacks a table the
    /// current schema declares (§6): recreate it, the draft offers no
    /// migration.
    Stale {
        /// The first table found missing.
        table: &'static str,
    },
    /// The store's `hash_algo` is not one this crate computes, or not the
    /// one the caller declared (§5).
    HashAlgo {
        /// The algorithm the store records.
        found: String,
        /// The algorithm the caller declared, when it declared one.
        declared: Option<&'static str>,
    },
    /// Another writer holds the write lock (§8); retry once it releases.
    Busy,
    /// Another process owns the store (§8), reported at once rather than
    /// waited out.
    Owned(PathBuf),
    /// A producer is between its blob write and the enqueue that pins it
    /// (§8), so a collector cannot run.
    Staging(PathBuf),
}

impl fmt::Display for PimdirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sql(err) => write!(f, "Pimdir SQL error: {err}"),
            Self::Io(err) => write!(f, "Pimdir I/O error: {err}"),
            Self::Json(err) => write!(f, "Pimdir JSON error: {err}"),
            Self::Action(err) => write!(f, "Pimdir action error: {err}"),
            Self::Rebind {
                collection,
                link_id,
                source,
                bound,
                incoming,
            } => write!(
                f,
                "Pimdir binding {collection}/{link_id} on source {source} holds handle {bound} and this write carries {incoming}: a binding pins one handle"
            ),
            Self::Version { found } => write!(
                f,
                "Pimdir store schema version {found} is unsupported (this crate services version {})",
                sql::VERSION
            ),
            Self::Uncreated => write!(
                f,
                "Pimdir store has no schema yet: its owner has to create it first"
            ),
            Self::VersionMismatch {
                user_version,
                store_meta,
            } => write!(
                f,
                "Pimdir store is corrupt: PRAGMA user_version is {user_version} but store_meta records {store_meta}"
            ),
            Self::Stale { table } => write!(
                f,
                "Pimdir store was written by an earlier draft and lacks the {table} table: delete the store and let it resync"
            ),
            Self::HashAlgo {
                found,
                declared: Some(declared),
            } => write!(
                f,
                "Pimdir store names its objects with {found}, not the {declared} this handle declared"
            ),
            Self::HashAlgo { found, .. } => write!(
                f,
                "Pimdir store names its objects with {found}, which this crate does not compute"
            ),
            Self::Busy => write!(
                f,
                "Pimdir store is busy: another writer holds the write lock, retry once it releases"
            ),
            Self::Owned(store) => write!(
                f,
                "Pimdir store at {} is owned by another process",
                store.display()
            ),
            Self::Staging(store) => write!(
                f,
                "Pimdir store at {} has a producer staging a body",
                store.display()
            ),
        }
    }
}

impl std::error::Error for PimdirError {}

impl From<rusqlite::Error> for PimdirError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Sql(err)
    }
}

impl From<io::Error> for PimdirError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<serde_json::Error> for PimdirError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

impl From<PimdirActionError> for PimdirError {
    fn from(err: PimdirActionError) -> Self {
        Self::Action(err)
    }
}
