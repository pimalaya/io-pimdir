//! [`PimdirStore`]: the std store that services [`io_replica`]'s storage seam.
//!
//! It persists a [`ReplicaHub`] per collection — one shared item plus a base per
//! source — and splits by whether an operation has a side at all:
//! [`PimdirStore`] is the store itself (the client reads, retention, the queue),
//! and [`PimdirSourceStore`], which [`for_source`] yields, services
//! [`ReplicaStorage`] for one source: `load` projects the hub for that source,
//! `write` absorbs the source's writes back. A single-source store is the N=1
//! case (one binding per item). Unlinked, freshly probed placements have no link
//! id to key an item on yet, so they are held in-memory as a residual until a
//! `Meta` upgrade resolves their link id.
//!
//! [`for_source`]: PimdirStore::for_source
//!
//! [`ReplicaStorage`]: io_replica::client::ReplicaStorage

use alloc::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use core::sync::atomic::{AtomicU64, Ordering};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt, fs,
    io::{self, ErrorKind, Write},
    mem,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
};

use io_replica::{
    change::ReplicaWriteOp,
    client::ReplicaStorage,
    collection::{ReplicaCheckpoint, ReplicaCollectionId},
    coroutine::{ReplicaArg, ReplicaCoroutine, ReplicaCoroutineState, ReplicaYield},
    hub::{ReplicaHub, ReplicaHubConflict, ReplicaHubItem, ReplicaSourceBinding, ReplicaSourceId},
    mutate::{ReplicaMutate, ReplicaMutation},
    object::{ReplicaHash, ReplicaObject},
    placement::{
        ReplicaBase, ReplicaFlags, ReplicaHandle, ReplicaLevel, ReplicaLinkId, ReplicaMeta,
        ReplicaPlacement, ReplicaSortKey, ReplicaStatus,
    },
    storage::{ReplicaLoadScope, ReplicaLoaded},
};
use rusqlite::{
    Connection, ErrorCode, OpenFlags, OptionalExtension, Row, TransactionBehavior, named_params,
    params,
};

use crate::{
    codec::{self, PimdirAction, PimdirActionError},
    hash::{PimdirHashAlgo, PimdirHasher},
    sql,
};

/// A pimdir store: the database and the blob directory, opened without naming
/// a side.
///
/// It carries what an operation means for the store as a whole — every client
/// read, retention and purge, the queue rows a cancellation removes — none of
/// which consults a source. The sync seam does, and lives on
/// [`PimdirSourceStore`], which [`for_source`](Self::for_source) yields.
pub struct PimdirStore {
    conn: Connection,
    blobs: PathBuf,
    /// The hash this store names its objects by (spec §5), read back from
    /// `store_meta.hash_algo` so every body a consumer hashes lands under the
    /// name the store already uses.
    hash: PimdirHashAlgo,
    /// The account every collection this handle creates belongs to (spec §9.2);
    /// `None` in a single-account store. Set with
    /// [`for_account`](PimdirStore::for_account).
    account: Option<String>,
}

/// A pimdir store acting as one source (`"left"`, `"right"`, `"phone"`, …):
/// the sync seam, where every operation means "as this side".
///
/// The underlying database and blobs are shared; several sources of one store
/// are several handles over the same files. Dereferences to the
/// [`PimdirStore`] it was made from, so the source-less surface stays reachable
/// through it.
pub struct PimdirSourceStore {
    store: PimdirStore,
    source: ReplicaSourceId,
    /// Unlinked probed placements, awaiting the `Meta` upgrade that gives them
    /// a link id; kept in memory (empty at rest between syncs).
    ///
    /// Keyed rather than listed: a first sync probes a whole collection before
    /// linking any of it, so the residual grows to the collection size while
    /// every insertion, every drop and every lookup searches it.
    residual: HashMap<(ReplicaCollectionId, ReplicaHandle), ReplicaPlacement>,
}

/// A collection as seen by a client read (`list_collections`): its identity and
/// presentation, kind-agnostic. The sync bindings and per-source state are not
/// exposed here — a reader observes the shared truth only.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirCollection {
    /// The stable collection id (the mailbox name for a mail store).
    pub id: String,
    /// The account this collection is grouped under (spec §9.2), `None` in a
    /// single-account store. It groups and nothing more: no identifier is
    /// scoped by it.
    pub account: Option<String>,
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
    /// The handle-space epoch (spec §12): starts at 1, bumped by the owner only
    /// on a handle-space rebuild (rekey), so a frontend derives epoch-dependent
    /// protocol values (an IMAP UIDVALIDITY) from the store alone.
    pub generation: i64,
}

/// Where one identity or one body sits, as the multiplicity reads report it
/// (spec §9.2): one row per live placement, carrying the collection and account
/// it occurs in.
///
/// A fact, not a verdict. The same vCard `UID` in two accounts' address books
/// is two of these; whether that is one person shown twice or two people is the
/// consumer's call, and the store never makes it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirPlacement {
    /// The collection the placement sits in.
    pub collection: String,
    /// The account that collection is grouped under, `None` when ungrouped.
    pub account: Option<String>,
    /// The item's public id, shared by every placement of one link id.
    pub seq: i64,
    /// The cross-collection identity.
    pub link_id: String,
    /// The body this placement points at, absent until hydrated.
    pub object: Option<ReplicaHash>,
    /// The placement's flags, as the stored JSON array.
    pub flags: Option<String>,
    /// The detail level: 0 probed, 1 meta, 2 full.
    pub level: i64,
}

/// Maps a `LIST_COLLECTIONS`-shaped row.
fn collection_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<PimdirCollection> {
    Ok(PimdirCollection {
        id: r.get(0)?,
        account: r.get(1)?,
        kind: r.get(2)?,
        name: r.get(3)?,
        parent: r.get(4)?,
        color: r.get(5)?,
        description: r.get(6)?,
        sort_order: r.get(7)?,
        generation: r.get(8)?,
    })
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
    /// The kind's ordering key (spec §9.3): a normalised RFC 3339 instant for
    /// mail and calendars, a normalised display name for contacts. Empty means
    /// unknown, which sorts before every real key ascending and after every one
    /// descending.
    pub sort_key: String,
    /// The content-addressed body hash; `None` until a `Full` hydrate.
    pub object: Option<ReplicaHash>,
    /// The detail tier the item is hydrated to.
    pub level: ReplicaLevel,
}

/// One retained (soft-deleted) item, as the trash view reads it
/// (`list_retained`): the whole row retention kept, body pointer and size
/// included, so a caller can show it, restore it or price a purge without a
/// second query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirRetainedItem {
    /// The message's public id, the same it held while live: a restore keeps
    /// it, and it is what `purge` addresses.
    pub seq: i64,
    /// The cross-source link id the retained row still holds.
    pub link_id: String,
    /// The flag set as of the moment the last binding vanished.
    pub flags: ReplicaFlags,
    /// The detail tier the item was hydrated to.
    pub level: ReplicaLevel,
    /// The raw per-domain summary blob, verbatim.
    pub meta: Option<String>,
    /// The kind's ordering key as of retirement, so a trash view can present
    /// its rows in the same order the live listing uses.
    pub sort_key: String,
    /// The body hash the row still pins; `None` when the item was never
    /// hydrated (nothing to reclaim, nothing to restore locally).
    pub object_hash: Option<String>,
    /// The body's size in bytes; `None` alongside an absent `object_hash`.
    pub size: Option<u64>,
    /// The RFC 3339 instant the **last binding vanished** (not when a server
    /// deleted the item, which is unknowable). A revive clears it, so
    /// restore-then-redelete restarts the purge clock.
    pub retained_at: String,
    /// The source whose removal retired the item; diagnostic, nothing keys on
    /// it.
    pub retained_by: Option<String>,
}

/// What a purge reclaimed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PimdirPurgeReport {
    /// Retained items deleted.
    pub items: usize,
    /// Blob bytes actually unlinked; a body another item still references is
    /// not counted, since it was not reclaimed.
    pub bytes: u64,
}

/// One pending (non-parked) queue row, in append order (spec §15.4): what a
/// frontend overlays on its item projection for read-your-writes, and what the
/// owner's drain applies.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirPendingAction {
    /// The row's global append id (`queue.id`).
    pub id: i64,
    /// The producer-supplied RFC 3339 enqueue timestamp.
    pub created_at: String,
    /// The enqueuing process, diagnostic only.
    pub producer: String,
    /// The decoded action.
    pub action: PimdirAction,
    /// Apply attempts so far.
    pub attempts: i64,
}

/// One parked queue row: an action the owner judged permanently unappliable,
/// recorded and skipped instead of blocking its collection's queue. Left for
/// operators and status surfaces, never silently deleted (spec §15.2). The
/// payload stays raw, since being undecodable may be why the row parked.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirParkedAction {
    /// The row's global append id (`queue.id`).
    pub id: i64,
    /// The producer-supplied RFC 3339 enqueue timestamp.
    pub created_at: String,
    /// The enqueuing process, diagnostic only.
    pub producer: String,
    /// The target collection.
    pub collection: String,
    /// The raw action kind.
    pub action: String,
    /// The raw versioned JSON payload.
    pub payload: String,
    /// Apply attempts before parking.
    pub attempts: i64,
    /// The failure that parked the row.
    pub error: String,
}

/// What a [`drain_collection`](PimdirStore::drain_collection) pass did.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PimdirDrainReport {
    /// Actions applied to the store and deleted from the queue.
    pub applied: usize,
    /// Actions parked with an error, left queryable.
    pub parked: usize,
    /// Actions this owner could not perform, left **pending** for one that can
    /// (spec §15.2). Not a failure: parking would claim the action is
    /// permanently unappliable, which is a different and wrong statement.
    pub skipped: usize,
}

impl PimdirStore {
    /// Opens (creating if absent) the store rooted at `dir`.
    ///
    /// A fresh database is created at the current schema version. A store
    /// stamped with a *higher* `user_version` than this crate services is
    /// refused with [`PimdirError::Version`] rather than half-read; the spec
    /// is a draft, so such a store is recreated, never migrated.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, PimdirError> {
        Self::open_with_hash(dir, None)
    }

    /// Opens (creating if absent) the store rooted at `dir`, declaring the hash
    /// its objects are named by (spec §5).
    ///
    /// A store records its algorithm once, at creation, in
    /// `store_meta.hash_algo`: every blob is a file named by it, so it cannot
    /// change afterwards. `hash` therefore applies to a store this call
    /// creates, and an existing store whose algorithm differs is refused with
    /// [`PimdirError::HashAlgo`] rather than opened into a handle that would
    /// hash bodies to names it does not use. Passing `None` adopts whatever the
    /// store records, and creates with [`PimdirHashAlgo::default`].
    ///
    /// A consumer hashes through [`hash`](Self::hash) or
    /// [`hasher`](Self::hasher) rather than choosing an algorithm of its own,
    /// which is what keeps two implementations of one store (this crate and the
    /// Android app's SQLite driver) naming the same body the same way.
    pub fn open_with_hash(
        dir: impl AsRef<Path>,
        hash: Option<PimdirHashAlgo>,
    ) -> Result<Self, PimdirError> {
        let dir = dir.as_ref();
        fs::create_dir_all(dir)?;
        let blobs = dir.join("objects");
        fs::create_dir_all(&blobs)?;

        let mut conn = Connection::open(dir.join("pimdir.db"))?;
        // NOTE: `busy_timeout` lets several handles of one store wait out each
        // other's write transaction instead of failing with `SQLITE_BUSY` — §8's
        // single-owner process opening `"left"` and `"right"`, and a sync that
        // fans work across several same-source handles (one per worker) to overlap
        // network while the writes serialise. 30s absorbs a burst of large writes
        // (a first sync's per-mailbox meta insert) contending on the write lock.
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 30000;",
        )?;
        init_schema(&mut conn, hash.unwrap_or_default())?;
        let hash = read_hash_algo(&conn, hash)?;

        Ok(Self {
            conn,
            blobs,
            hash,
            account: None,
        })
    }

    /// Opens an **existing** store rooted at `dir` read-only.
    ///
    /// The database is opened with `SQLITE_OPEN_READ_ONLY`: nothing is
    /// created, so a missing database errors and a schema version other than
    /// the current one is refused with [`PimdirError::Version`] (a reader's
    /// SQL requires the current columns and never creates the schema; that is
    /// the owner's opening write). The returned
    /// handle exposes the full read surface; any write through it fails at the
    /// SQLite layer.
    pub fn open_read_only(dir: impl AsRef<Path>) -> Result<Self, PimdirError> {
        let dir = dir.as_ref();
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(dir.join("pimdir.db"), flags)?;
        conn.execute_batch("PRAGMA busy_timeout = 30000;")?;

        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version != sql::VERSION {
            return Err(PimdirError::Version { found: version });
        }
        check_version_agreement(&conn, version)?;
        check_rename_cascades(&conn)?;
        let hash = read_hash_algo(&conn, None)?;

        Ok(Self {
            conn,
            blobs: dir.join("objects"),
            hash,
            account: None,
        })
    }

    /// The hash this store names its objects by (spec §5).
    pub fn hash_algo(&self) -> PimdirHashAlgo {
        self.hash
    }

    /// A blob handle over this store's object directory, bound to the hash the
    /// store names its bodies by.
    ///
    /// Independent of the SQLite connection, so a body can be read while the
    /// store is mutably borrowed servicing a sync.
    pub fn blobs(&self) -> PimdirBlobs {
        PimdirBlobs {
            root: self.blobs.clone(),
            hash: self.hash,
        }
    }

    /// The content hash of a whole body, under this store's algorithm.
    pub fn hash(&self, bytes: &[u8]) -> ReplicaHash {
        self.hash.hash(bytes)
    }

    /// An incremental hasher for a body streamed into the blob store rather
    /// than held whole in memory, paired with [`PimdirBlobs::writer`].
    pub fn hasher(&self) -> PimdirHasher {
        self.hash.hasher()
    }

    /// Binds this handle to an account, so every collection it creates is
    /// grouped under it (spec §9.2).
    ///
    /// A single-account store never calls this and its collections carry a
    /// `NULL` account, which is what every by-account read matches when given
    /// `None`. A multi-account owner opens one handle per account, the same way
    /// it already opens one per source; handles are a SQLite connection each,
    /// and §8's single-owner rule is unchanged by how many a process holds.
    ///
    /// The account groups and nothing more: it partitions no identifier, so two
    /// accounts holding one link id still share a `seq`, and one body reaching
    /// both is still stored once. Where an identity or a body occurs is
    /// reported by [`link_placements`](Self::link_placements) and
    /// [`object_placements`](Self::object_placements), for the consumer to
    /// interpret.
    pub fn for_account(mut self, account: impl Into<String>) -> Self {
        self.account = Some(account.into());
        self
    }

    /// The account this handle writes under, `None` in a single-account store.
    pub fn account(&self) -> Option<&str> {
        self.account.as_deref()
    }

    /// Binds this handle to a source, yielding the sync seam: `load` projects
    /// the hub for that side and `write` folds its decisions back.
    ///
    /// A source is a side this store syncs with (`"left"`, `"right"`,
    /// `"phone"`, …), so it is only ever named by an operation that acts as
    /// one. Everything else — the reads, retention, the queue — stays on the
    /// source-less handle and is still reachable through the returned one.
    pub fn for_source(self, source: impl Into<String>) -> PimdirSourceStore {
        PimdirSourceStore {
            store: self,
            source: ReplicaSourceId(source.into()),
            residual: HashMap::new(),
        }
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
    /// The collection is grouped under this handle's account
    /// ([`for_account`](Self::for_account)), and an existing row keeps the
    /// account it already had: only the kind is updated.
    pub fn ensure_collection(&self, collection: &str, kind: &str) -> Result<(), PimdirError> {
        self.conn.execute(
            sql::SET_COLLECTION_KIND,
            named_params! {
                ":collection": collection,
                ":account": self.account.as_deref(),
                ":kind": kind,
            },
        )?;
        Ok(())
    }

    /// Regroups a collection under `account`, or out of one with `None`.
    ///
    /// Safe at any time: the account partitions no identifier (spec §9.2), so
    /// the move leaves the collection's `seq`s, link ids and objects alone.
    pub fn set_collection_account(
        &self,
        collection: &str,
        account: Option<&str>,
    ) -> Result<(), PimdirError> {
        self.conn.execute(
            sql::SET_COLLECTION_ACCOUNT,
            named_params! { ":collection": collection, ":account": account },
        )?;
        Ok(())
    }

    /// The account a collection is grouped under.
    ///
    /// The outer `Option` is "does the collection exist", the inner one "is it
    /// grouped": `Ok(None)` for an unknown collection, `Ok(Some(None))` for one
    /// in a single-account store.
    pub fn collection_account(
        &self,
        collection: &str,
    ) -> Result<Option<Option<String>>, PimdirError> {
        Ok(self
            .conn
            .query_row(
                sql::LOAD_ACCOUNT,
                named_params! { ":collection": collection },
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?)
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
        let rows = stmt.query_map([], collection_row)?;
        let mut collections = Vec::new();
        for row in rows {
            collections.push(row?);
        }
        Ok(collections)
    }

    /// Lists one account's collections, the filter axis of a merged view
    /// (spec §9.2).
    ///
    /// `None` selects the collections of a single-account store, matching on
    /// `IS` so a `NULL` account matches itself; `=` would match nothing.
    pub fn list_collections_by_account(
        &self,
        account: Option<&str>,
    ) -> Result<Vec<PimdirCollection>, PimdirError> {
        let mut stmt = self.conn.prepare(sql::LIST_COLLECTIONS_BY_ACCOUNT)?;
        let rows = stmt.query_map(named_params! { ":account": account }, collection_row)?;
        let mut collections = Vec::new();
        for row in rows {
            collections.push(row?);
        }
        Ok(collections)
    }

    /// The accounts owning at least one collection.
    ///
    /// Not a configured roster: a store learns an account only through its
    /// collections (spec §9.2), so an account with none yet does not appear
    /// here and a consumer holding the real roster reads it from its own
    /// configuration.
    pub fn list_accounts(&self) -> Result<Vec<String>, PimdirError> {
        let mut stmt = self.conn.prepare(sql::LIST_ACCOUNTS)?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut accounts = Vec::new();
        for row in rows {
            accounts.push(row?);
        }
        Ok(accounts)
    }

    /// Every live placement of one identity, with the collection and account it
    /// sits in (spec §9.2).
    ///
    /// The store reports where a link id occurs and takes no position on
    /// whether the placements are one thing. A mail view lists them, because
    /// two receipts of a newsletter have two read states and two servers; a
    /// contact view may offer to merge them, because one person in two address
    /// books is usually one person. Both read these rows.
    pub fn link_placements(&self, link_id: &str) -> Result<Vec<PimdirPlacement>, PimdirError> {
        let mut stmt = self.conn.prepare(sql::LIST_LINK_PLACEMENTS)?;
        let rows = stmt.query_map(named_params! { ":link_id": link_id }, |r| {
            Ok(PimdirPlacement {
                collection: r.get(0)?,
                account: r.get(1)?,
                seq: r.get(2)?,
                link_id: link_id.to_string(),
                object: r.get::<_, Option<String>>(3)?.map(ReplicaHash),
                flags: r.get(4)?,
                level: r.get(5)?,
            })
        })?;
        let mut placements = Vec::new();
        for row in rows {
            placements.push(row?);
        }
        Ok(placements)
    }

    /// Every live placement of one body, by content hash: the dedup axis rather
    /// than the identity one, so it pairs placements two servers gave different
    /// link ids.
    pub fn object_placements(&self, hash: &str) -> Result<Vec<PimdirPlacement>, PimdirError> {
        let mut stmt = self.conn.prepare(sql::LIST_OBJECT_PLACEMENTS)?;
        let rows = stmt.query_map(named_params! { ":hash": hash }, |r| {
            Ok(PimdirPlacement {
                collection: r.get(0)?,
                account: r.get(1)?,
                seq: r.get(2)?,
                link_id: r.get(3)?,
                object: Some(ReplicaHash(hash.to_string())),
                flags: r.get(4)?,
                level: r.get(5)?,
            })
        })?;
        let mut placements = Vec::new();
        for row in rows {
            placements.push(row?);
        }
        Ok(placements)
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

    /// A keyset page of a collection's live items in the kind's own **ascending**
    /// order (spec §9.3): A to Z for contacts, earliest first for mail and
    /// calendars.
    ///
    /// `after` is the previous page's last `(sort_key, seq)`; `None` starts from
    /// the beginning. The pair is the cursor because a sort key is not unique
    /// (two messages share a timestamp, two contacts share a name) and `seq`,
    /// unique per collection, is what makes the page total: no item is skipped
    /// or repeated across a boundary.
    pub fn list_items_page_asc(
        &self,
        collection: &str,
        after: Option<(&str, i64)>,
        limit: usize,
    ) -> Result<Vec<PimdirItem>, PimdirError> {
        // No real key sorts before an unknown one ascending, so the empty
        // string with seq 0 is the true beginning rather than a sentinel.
        // NOTE: no cursor ascending is the empty key, which is a real
        // one: an unknown key sorts first, so the page starts at it.
        let (key, seq) = after.unwrap_or(("", 0));
        self.sorted_page(
            sql::LIST_ITEMS_PAGE_ASC,
            collection,
            Some((key, seq)),
            limit,
        )
    }

    /// The same page **descending**: newest first for mail and calendars, Z to A
    /// for contacts.
    ///
    /// `None` starts from the end, which the statement expresses by binding a
    /// key above every representable one; a caller never has to invent that
    /// sentinel itself.
    pub fn list_items_page_desc(
        &self,
        collection: &str,
        after: Option<(&str, i64)>,
        limit: usize,
    ) -> Result<Vec<PimdirItem>, PimdirError> {
        self.sorted_page(sql::LIST_ITEMS_PAGE_DESC, collection, after, limit)
    }

    fn sorted_page(
        &self,
        statement: &str,
        collection: &str,
        after: Option<(&str, i64)>,
        limit: usize,
    ) -> Result<Vec<PimdirItem>, PimdirError> {
        let mut stmt = self.conn.prepare(statement)?;
        let rows = stmt.query_map(
            named_params! {
                ":collection": collection,
                ":after_key": after.map(|(key, _)| key),
                ":after_seq": after.map(|(_, seq)| seq).unwrap_or_default(),
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

    /// Restates one item's ordering key (spec §9.3).
    ///
    /// For a re-projection: a store written before its kind had a sort-key
    /// convention, one whose convention changed, or a consumer whose sync engine
    /// does not carry the key inline yet and derives it from the `meta` it wrote
    /// itself. Not part of the ordinary write path, which preserves an existing
    /// key by never naming it.
    pub fn set_sort_key(
        &self,
        collection: &str,
        link_id: &str,
        sort_key: &str,
    ) -> Result<(), PimdirError> {
        self.conn.execute(
            sql::SET_SORT_KEY,
            named_params! {
                ":collection": collection,
                ":link_id": link_id,
                ":sort_key": sort_key,
            },
        )?;
        Ok(())
    }

    /// Gives a collection a new id, carrying its whole contents with it.
    ///
    /// Every foreign key onto `collections(id)` is `ON UPDATE CASCADE`, so the
    /// items, bindings, sources, queue rows and child collections follow in the
    /// same statement (spec §14). This is the **only** safe way to change an id:
    /// deleting the collection and recreating it under the new one destroys the
    /// cache, because `ON DELETE CASCADE` takes every item and binding with it,
    /// turning a rename into a full re-download and discarding any staged local
    /// change not yet pushed.
    ///
    /// Two things make an id change: a server renaming the collection (an IMAP
    /// `RENAME`, a DAV move), and an owner renaming an account whose id it
    /// namespaced its collection ids with. An account rename is one call per
    /// collection of that account; run them in one transaction and the account
    /// moves atomically or not at all.
    pub fn rename_collection(&self, collection: &str, new_id: &str) -> Result<(), PimdirError> {
        self.conn.execute(
            sql::RENAME_COLLECTION,
            named_params! { ":collection": collection, ":new_id": new_id },
        )?;
        Ok(())
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

/// The retention surface (spec §11): the trash a store keeps instead of losing
/// items, and the only operations that truly destroy one.
///
/// An item whose last source binding vanished is retained, not deleted: hidden
/// from the sync seam (so no sync ever re-derives it) and from the live client
/// reads, but kept whole, body included. It comes back either by revival (its
/// link id reappears, whether from a source or from a client `add`) or not at
/// all, until a purge reclaims it. Retention is unconditional; *when* to
/// reclaim is the owner's schedule, which is why every purge takes its
/// boundary from the caller.
impl PimdirStore {
    /// A keyset page of a collection's retained items.
    ///
    /// `after` is the exclusive lower bound on the public `seq` (`None` starts
    /// from the beginning); at most `limit` items are returned, ordered by
    /// `seq`, so the last item's [`seq`](PimdirRetainedItem::seq) is the cursor
    /// for the next page. This is the only read that returns retained items: a
    /// caller presents them as a trash view, never merged into the live listing.
    pub fn list_retained(
        &self,
        collection: &ReplicaCollectionId,
        after: Option<i64>,
        limit: usize,
    ) -> Result<Vec<PimdirRetainedItem>, PimdirError> {
        let mut stmt = self.conn.prepare(sql::LIST_RETAINED_PAGE)?;
        let rows = stmt.query_map(
            named_params! {
                ":collection": collection.0,
                ":after": after.unwrap_or(0),
                ":limit": limit as i64,
            },
            |row| {
                let size: Option<i64> = row.get(9)?;
                Ok(PimdirRetainedItem {
                    seq: row.get(0)?,
                    link_id: row.get(1)?,
                    flags: codec::flags_from_json(row.get::<_, Option<String>>(2)?.as_deref()),
                    object_hash: row.get(3)?,
                    meta: row.get(4)?,
                    sort_key: row.get(5)?,
                    level: codec::level_from_int(row.get(6)?),
                    retained_at: row.get(7)?,
                    retained_by: row.get(8)?,
                    size: size.map(|size| size.max(0) as u64),
                })
            },
        )?;
        let mut items = Vec::new();
        for row in rows {
            items.push(row?);
        }
        Ok(items)
    }

    /// A collection's retained item count, the counterpart of
    /// [`count_items`](Self::count_items).
    pub fn count_retained(&self, collection: &ReplicaCollectionId) -> Result<i64, PimdirError> {
        Ok(self.conn.query_row(
            sql::COUNT_RETAINED,
            named_params! { ":collection": collection.0 },
            |r| r.get(0),
        )?)
    }

    /// The bytes retention is holding across the whole store, each distinct body
    /// counted once.
    ///
    /// An **upper bound** on what a purge would reclaim: a body a live item also
    /// points at keeps that reference and survives the sweep. Reported so an
    /// operator can price a retention duration before choosing one.
    pub fn retained_bytes(&self) -> Result<u64, PimdirError> {
        let bytes: i64 = self.conn.query_row(sql::RETAINED_BYTES, [], |r| r.get(0))?;
        Ok(bytes.max(0) as u64)
    }

    /// Purges one retained item by its public id, returning whether there was
    /// one to purge.
    ///
    /// The row goes, its bindings cascade, and the body it released is unlinked
    /// by the ordinary refcount sweep once nothing else references it: a purge
    /// runs no garbage collection of its own. A **live** item is never reached
    /// by this (the statement is guarded on the retention stamp), so an
    /// operator emptying the trash cannot destroy synced data.
    pub fn purge(
        &mut self,
        collection: &ReplicaCollectionId,
        seq: i64,
    ) -> Result<bool, PimdirError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;

        let pinned: Option<(Option<String>, Option<String>)> = tx
            .query_row(
                sql::RETAINED_ITEM_BY_SEQ,
                named_params! { ":collection": collection.0, ":seq": seq },
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((object, conflict_object)) = pinned else {
            return Ok(false);
        };

        tx.execute(
            sql::PURGE_ITEM,
            named_params! { ":collection": collection.0, ":seq": seq },
        )?;
        release_pins(&tx, [object, conflict_object].into_iter().flatten())?;
        let garbage = collect_garbage(&tx)?;
        tx.commit().map_err(busy_or_sql)?;

        for (hash, _) in garbage {
            remove_blob(&self.blobs, &hash)?;
        }
        Ok(true)
    }

    /// The scheduled sweep: purges every item retired **strictly before**
    /// `cutoff` (RFC 3339), store-wide, reporting what it reclaimed.
    ///
    /// The boundary is the caller's, not the store's clock: an owner computes it
    /// from its own retention duration, so the store holds no policy and the
    /// sweep stays deterministic even though the stamps are SQLite's. An item
    /// retained exactly at `cutoff` is kept. A cutoff of *now* reproduces the
    /// terminal-delete behaviour of a store that never retained, which is why
    /// there is no on/off switch.
    pub fn purge_retained_before(
        &mut self,
        cutoff: &str,
    ) -> Result<PimdirPurgeReport, PimdirError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;

        let pinned: Vec<(Option<String>, Option<String>)> = {
            let mut stmt = tx.prepare(sql::RETAINED_BEFORE)?;
            let rows = stmt.query_map(named_params! { ":cutoff": cutoff }, |row| {
                Ok((row.get(2)?, row.get(3)?))
            })?;
            let mut pinned = Vec::new();
            for row in rows {
                pinned.push(row?);
            }
            pinned
        };
        let items = pinned.len();
        tx.execute(
            sql::PURGE_RETAINED_BEFORE,
            named_params! { ":cutoff": cutoff },
        )?;
        release_pins(
            &tx,
            pinned
                .into_iter()
                .flat_map(|(object, conflict)| [object, conflict])
                .flatten(),
        )?;
        let garbage = collect_garbage(&tx)?;
        tx.commit().map_err(busy_or_sql)?;

        // NOTE: the bytes are the blobs actually unlinked, so a body another
        // item still references is not claimed as reclaimed.
        let mut bytes = 0;
        for (hash, size) in garbage {
            remove_blob(&self.blobs, &hash)?;
            bytes += size;
        }
        Ok(PimdirPurgeReport { items, bytes })
    }
}

/// Releases the object references a retained row (or a queue row) held, so the
/// ordinary sweep can reclaim a body nothing points at any more.
fn release_pins(
    conn: &Connection,
    hashes: impl Iterator<Item = String>,
) -> Result<(), PimdirError> {
    // NOTE: one statement rather than one per hash. A purge sweeping fifty
    // thousand retained items releases two pins each, and a point update per
    // pin is a hundred thousand statements inside one transaction to express
    // a set operation.
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

/// The action-queue owner surface (spec §15) and collection generations (spec
/// §12): the single owning process drains producer-requested mutations into the
/// store, and marks a handle-space rebuild for readers.
impl PimdirStore {
    /// A collection's handle-space epoch (spec §12), or `None` when the store
    /// has never seen the collection. Starts at 1; bumped only by
    /// [`write_rekeyed`](Self::write_rekeyed), so a frontend derives
    /// epoch-dependent protocol values (an IMAP UIDVALIDITY) from it alone.
    pub fn generation(&self, collection: &str) -> Result<Option<i64>, PimdirError> {
        Ok(self
            .conn
            .query_row(
                sql::LOAD_GENERATION,
                named_params! { ":collection": collection },
                |r| r.get(0),
            )
            .optional()?)
    }

    /// The collections with pending (non-parked) queue work, for the owner's
    /// drain loop.
    pub fn queued_collections(&self) -> Result<Vec<String>, PimdirError> {
        let mut stmt = self.conn.prepare(sql::LIST_QUEUED_COLLECTIONS)?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut collections = Vec::new();
        for row in rows {
            collections.push(row?);
        }
        Ok(collections)
    }

    /// A collection's pending (non-parked) actions in append order, decoded
    /// (read surface, spec §15.4): a frontend overlays them on its item
    /// projection for read-your-writes. An undecodable payload errors; the
    /// owner's next drain parks such a row.
    pub fn pending_actions(
        &self,
        collection: &str,
    ) -> Result<Vec<PimdirPendingAction>, PimdirError> {
        load_pending_actions(&self.conn, collection)
    }

    /// Every parked action across the store, in append order, for status
    /// surfaces and operator repair. Parked rows are skipped by the drain and
    /// never silently deleted.
    pub fn parked_actions(&self) -> Result<Vec<PimdirParkedAction>, PimdirError> {
        let mut stmt = self.conn.prepare(sql::LOAD_PARKED_ACTIONS)?;
        let rows = stmt.query_map([], |r| {
            Ok(PimdirParkedAction {
                id: r.get(0)?,
                created_at: r.get(1)?,
                producer: r.get(2)?,
                collection: r.get(3)?,
                action: r.get(4)?,
                payload: r.get(5)?,
                attempts: r.get(6)?,
                error: r.get(7)?,
            })
        })?;
        let mut actions = Vec::new();
        for row in rows {
            actions.push(row?);
        }
        Ok(actions)
    }

    /// Removes one queue row by request rather than by application, pending or
    /// parked, returning whether there was a row to remove (spec §15.5).
    ///
    /// One verb for the two ways a row leaves the queue unapplied: a producer
    /// (or an operator) **cancelling** a queued action, and an owner
    /// **acknowledging** an intent it performed out of band, which the drain
    /// could only skip. The row's body pin is released in the same transaction,
    /// so a blob nothing else references falls to the ordinary sweep.
    pub fn drop_action(&mut self, id: i64) -> Result<bool, PimdirError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;

        let hash: Option<Option<String>> = tx
            .query_row(sql::LOAD_ACTION_ROW, named_params! { ":id": id }, |r| {
                r.get(1)
            })
            .optional()?;
        let Some(hash) = hash else {
            return Ok(false);
        };

        tx.execute(sql::CANCEL_ACTION, named_params! { ":id": id })?;
        release_pins(&tx, hash.into_iter())?;
        let garbage = collect_garbage(&tx)?;
        tx.commit().map_err(busy_or_sql)?;

        for (hash, _) in garbage {
            remove_blob(&self.blobs, &hash)?;
        }
        Ok(true)
    }

    /// Records a failed apply an owner performed itself (spec §15.2).
    ///
    /// `None` is the transient case: the attempt counter advances and the row
    /// stays pending, so the next drain picks it up again. `Some(error)` is the
    /// permanent one: the row parks with the failure, visible to operators
    /// instead of blocking its collection forever. An unknown id is a no-op,
    /// since the row may have been applied or cancelled in between.
    pub fn fail_action(&mut self, id: i64, error: Option<&str>) -> Result<(), PimdirError> {
        let Some(error) = error else {
            self.conn
                .execute(sql::BUMP_ATTEMPTS, named_params! { ":id": id })?;
            return Ok(());
        };

        let attempts: Option<i64> = self
            .conn
            .query_row(sql::LOAD_ACTION_ROW, named_params! { ":id": id }, |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(attempts) = attempts {
            self.conn.execute(
                sql::PARK_ACTION,
                named_params! { ":id": id, ":attempts": attempts + 1, ":error": error },
            )?;
        }
        Ok(())
    }
}

/// The sync seam and what only a side can mean: the source-bound writes, and
/// the drain that stages a producer's queued mutation for that side.
impl PimdirSourceStore {
    /// The source this handle acts as.
    pub fn source(&self) -> &str {
        &self.source.0
    }

    /// Binds this handle to an account, so every collection it creates is
    /// grouped under it (spec §9.2); see
    /// [`PimdirStore::for_account`], which this defers to so the two
    /// bindings can be given in either order.
    pub fn for_account(mut self, account: impl Into<String>) -> Self {
        self.store = self.store.for_account(account);
        self
    }

    /// Applies a handle-space rebuild's write batch and bumps the collection's
    /// generation **in the same transaction**, returning the new generation.
    ///
    /// The owner drives io-replica's rekey coroutine and routes its rebuild
    /// writes here instead of [`write`](ReplicaStorage::write), so "the ids you
    /// cached are void" commits atomically with the rebuild that voided them.
    /// Ordinary syncs, full resyncs from an expired checkpoint, and content
    /// changes never bump; they keep using `write`.
    pub fn write_rekeyed(
        &mut self,
        collection: &str,
        ops: Vec<ReplicaWriteOp>,
    ) -> Result<i64, PimdirError> {
        let tx = self
            .store
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        apply_ops(
            &tx,
            &self.store.blobs,
            &self.source,
            self.store.account.as_deref(),
            &mut self.residual,
            ops,
        )?;
        tx.execute(
            sql::ENSURE_COLLECTION,
            named_params! { ":collection": collection, ":account": self.store.account.as_deref() },
        )?;
        let generation: i64 = tx.query_row(
            sql::BUMP_GENERATION,
            named_params! { ":collection": collection },
            |r| r.get(0),
        )?;
        let garbage = collect_garbage(&tx)?;
        tx.commit().map_err(busy_or_sql)?;

        for (hash, _) in garbage {
            remove_blob(&self.store.blobs, &hash)?;
        }
        Ok(generation)
    }

    /// Drains a collection's pending actions in append order (spec §15.2).
    ///
    /// Each action is applied as the store mutation it names — resolving its
    /// public `seq` to the internal link id, staging the corresponding
    /// io-replica mutation and folding its writes through the store's own write
    /// machinery — and its row is deleted **in the same transaction**, so
    /// application is exactly-once and never partially visible. An action the
    /// owner judges permanently unappliable (malformed payload, unknown `seq`,
    /// duplicate `add` link id) is parked with its error and skipped without
    /// blocking later actions. A transient failure increments the row's
    /// `attempts` and stops the pass with the error, preserving apply order for
    /// the retry.
    ///
    /// An action whose kind this store defines no semantics for is **skipped**:
    /// left pending, never parked, never blocking the actions behind it. That
    /// is what lets one queue carry store mutations any owner applies beside
    /// capability-bound intents (a mail submission) only a specific owner can
    /// perform; that owner reads the row through
    /// [`pending_actions`](PimdirStore::pending_actions), performs it, and
    /// acknowledges it with [`drop_action`](PimdirStore::drop_action).
    pub fn drain_collection(&mut self, collection: &str) -> Result<PimdirDrainReport, PimdirError> {
        let rows: Vec<QueueRow> = {
            let mut stmt = self.store.conn.prepare(sql::LOAD_PENDING_ACTIONS)?;
            let rows = stmt.query_map(named_params! { ":collection": collection }, |r| {
                Ok(QueueRow {
                    id: r.get(0)?,
                    action: r.get(3)?,
                    payload: r.get(4)?,
                    object_hash: r.get(5)?,
                    attempts: r.get(6)?,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            out
        };

        let mut report = PimdirDrainReport::default();
        for row in rows {
            let action = match codec::action_from_payload(&row.action, &row.payload) {
                Ok(action) => action,
                Err(err) => {
                    self.park(&row, &err.to_string())?;
                    report.parked += 1;
                    continue;
                }
            };
            if matches!(action, PimdirAction::Unknown { .. }) {
                report.skipped += 1;
                continue;
            }
            match self.apply_queued(collection, &row, &action) {
                Ok(None) => report.applied += 1,
                Ok(Some(reason)) => {
                    self.park(&row, &reason)?;
                    report.parked += 1;
                }
                Err(err) => {
                    self.store
                        .conn
                        .execute(sql::BUMP_ATTEMPTS, named_params! { ":id": row.id })?;
                    return Err(err);
                }
            }
        }
        Ok(report)
    }

    /// Applies one queued action and deletes its row in one transaction,
    /// releasing the row's object pin as the applied item takes its own
    /// reference. Returns `Some(reason)` when the action must be parked (the
    /// transaction is rolled back), `None` when applied.
    fn apply_queued(
        &mut self,
        collection: &str,
        row: &QueueRow,
        action: &PimdirAction,
    ) -> Result<Option<String>, PimdirError> {
        let tx = self
            .store
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;

        // NOTE: claim the row before doing its work. The pending rows were
        // read outside any transaction, so another owner may hold the same
        // list and have applied this one already; `add` and `copy` would
        // then land twice. A claim that deletes nothing means exactly
        // that, and there is nothing left to do.
        let claimed = tx
            .prepare(sql::CLAIM_ACTION)?
            .query_row(named_params! { ":id": row.id }, |r| r.get::<_, i64>(0))
            .optional()?;
        if claimed.is_none() {
            return Ok(None);
        }

        let ops = match stage_action(&tx, &self.source, collection, row.id, action)? {
            Ok(ops) => ops,
            // NOTE: dropping the transaction rolls the attempt back.
            Err(reason) => return Ok(Some(reason)),
        };
        apply_ops(
            &tx,
            &self.store.blobs,
            &self.source,
            self.store.account.as_deref(),
            &mut self.residual,
            ops,
        )?;
        // NOTE: the incremental pin hand-over: the queue row's reference
        // (taken at enqueue) is released as the row goes, while the applied
        // item's own reference was just taken by `apply_ops`, all in this
        // transaction, so a queued body is never sweepable in between.
        if let Some(hash) = &row.object_hash {
            tx.execute(
                sql::ADJUST_REFCOUNT,
                named_params! { ":delta": -1, ":hash": hash },
            )?;
        }
        let garbage = collect_garbage(&tx)?;
        tx.commit().map_err(busy_or_sql)?;

        for (hash, _) in garbage {
            remove_blob(&self.store.blobs, &hash)?;
        }
        Ok(None)
    }
    /// Parks one queue row: records the failure and the spent attempt, leaving
    /// the row queryable and the rest of the queue flowing.
    fn park(&self, row: &QueueRow, error: &str) -> Result<(), PimdirError> {
        self.store.conn.execute(
            sql::PARK_ACTION,
            named_params! { ":id": row.id, ":attempts": row.attempts + 1, ":error": error },
        )?;
        Ok(())
    }
}

impl ReplicaStorage for PimdirSourceStore {
    type Error = PimdirError;

    fn load(
        &self,
        collection: &ReplicaCollectionId,
        scope: &ReplicaLoadScope,
    ) -> Result<ReplicaLoaded, Self::Error> {
        // NOTE: the scope narrows the hub read, and the projection then only
        // ever produces placements for what was read. A handle scope cannot
        // narrow the query, since the hub is keyed by link id and a handle
        // resolves to one only through a binding: it is resolved first, and a
        // handle no binding holds simply contributes nothing.
        let hub = match scope {
            ReplicaLoadScope::All => load_hub(&self.store.conn, &collection.0)?,
            ReplicaLoadScope::Links(links) => {
                let links: Vec<String> = links.iter().map(|l| l.0.clone()).collect();
                load_hub_by_link(&self.store.conn, &collection.0, &links)?
            }
            ReplicaLoadScope::Handles(handles) => {
                let mut links = Vec::new();
                for handle in handles {
                    let link = self
                        .store
                        .conn
                        .query_row(
                            sql::LINK_FOR_HANDLE,
                            named_params! {
                                ":collection": collection.0,
                                ":source": self.source.0,
                                ":handle": handle.0,
                            },
                            |r| r.get::<_, String>(0),
                        )
                        .optional()?;
                    links.extend(link);
                }
                load_hub_by_link(&self.store.conn, &collection.0, &links)?
            }
        };

        let mut placements = hub.project(collection, &self.source);
        placements.extend(
            self.residual
                .values()
                .filter(|p| &p.collection == collection)
                .cloned(),
        );

        let checkpoint = self
            .store
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
        let mut stmt = self.store.conn.prepare(sql::LOOKUP_OBJECTS)?;
        let rows = stmt.query_map(
            named_params! { ":links": json, ":account": self.store.account.as_deref() },
            |r| {
                Ok((
                    ReplicaLinkId(r.get::<_, String>(0)?),
                    ReplicaHash(r.get::<_, String>(1)?),
                ))
            },
        )?;
        for row in rows {
            let (link, hash) = row?;
            map.insert(link, hash);
        }

        // NOTE: a body hydrated on a not-yet-linked residual placement.
        for placement in self.residual.values() {
            if let (Some(link), Some(object)) = (&placement.link_id, &placement.object) {
                if links.contains(link) {
                    map.entry(link.clone()).or_insert_with(|| object.clone());
                }
            }
        }

        Ok(map)
    }

    fn write(&mut self, ops: Vec<ReplicaWriteOp>) -> Result<(), Self::Error> {
        // BEGIN IMMEDIATE takes the single writer lock up front (§8): under WAL
        // reads never block, but two writers serialise here, and a writer that
        // cannot get the lock within `busy_timeout` fails fast and loud (`Busy`)
        // rather than deep inside the batch on a deferred lock upgrade.
        let tx = self
            .store
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        apply_ops(
            &tx,
            &self.store.blobs,
            &self.source,
            self.store.account.as_deref(),
            &mut self.residual,
            ops,
        )?;
        let garbage = collect_garbage(&tx)?;
        tx.commit().map_err(busy_or_sql)?;

        for (hash, _) in garbage {
            remove_blob(&self.store.blobs, &hash)?;
        }
        Ok(())
    }
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

/// One raw pending queue row, as the drain loads it (the payload undecoded, so
/// a malformed one can be parked instead of failing the pass).
struct QueueRow {
    id: i64,
    action: String,
    payload: String,
    object_hash: Option<String>,
    attempts: i64,
}

/// Loads a collection's pending actions in append order, decoding each payload
/// strictly. Shared by [`PimdirStore::pending_actions`] and
/// [`PimdirProducer::pending_actions`].
fn load_pending_actions(
    conn: &Connection,
    collection: &str,
) -> Result<Vec<PimdirPendingAction>, PimdirError> {
    let mut stmt = conn.prepare(sql::LOAD_PENDING_ACTIONS)?;
    let rows = stmt.query_map(named_params! { ":collection": collection }, |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, i64>(6)?,
        ))
    })?;

    let mut actions = Vec::new();
    for row in rows {
        let (id, created_at, producer, kind, payload, attempts) = row?;
        actions.push(PimdirPendingAction {
            id,
            created_at,
            producer,
            action: codec::action_from_payload(&kind, &payload)?,
            attempts,
        });
    }
    Ok(actions)
}

/// Stages the io-replica write ops one queued action folds into the store
/// (spec §15.3), inside the drain transaction. The inner `Err` is a park
/// reason (the action is permanently unappliable); an empty op list is a
/// no-op success (a `remove` of an already-absent item).
///
/// Existing items are addressed by `seq`, resolved to their link id and then
/// to this source's projected placement; the matching [`ReplicaMutation`] is
/// then pumped through the real [`ReplicaMutate`] coroutine, so the staging
/// semantics (dirty/tombstone/created marking, conflict handling) stay the
/// engine's, not a re-implementation. An `add` is staged directly as the same
/// `Created` placement the engine's `Add` mutation stages, minus the body
/// bytes: the producer already wrote the blob and indexed the object at
/// enqueue.
fn stage_action(
    tx: &Connection,
    source: &ReplicaSourceId,
    collection: &str,
    row_id: i64,
    action: &PimdirAction,
) -> Result<Result<Vec<ReplicaWriteOp>, String>, PimdirError> {
    let collection_id = ReplicaCollectionId(collection.to_string());

    if let PimdirAction::Add {
        link_id,
        flags,
        object,
        meta,
        handle,
    } = action
    {
        let link = link_id
            .clone()
            .or_else(|| object.as_ref().map(|hash| ReplicaLinkId(hash.0.clone())));
        let Some(link) = link else {
            return Ok(Err("add carries neither link_id nor object".to_string()));
        };
        // NOTE: the same collision rule as the engine's Add mutation — a live
        // item blocks the create, a tombstone does not (the delete is in
        // flight; the new item supersedes it). Asked of the one row that could
        // collide, not of the collection: this runs once per drained action,
        // so loading the whole collection to answer it makes a drain of N
        // actions cost N passes over the mailbox.
        let live = tx
            .query_row(
                sql::LIVE_ITEM_FOR_LINK,
                named_params! { ":collection": collection, ":link_id": link.0 },
                |r| r.get::<_, i64>(0),
            )
            .optional()?;
        if live.is_some() {
            return Ok(Err(format!("link id already present: {}", link.0)));
        }
        let level = match (object, meta) {
            (Some(_), _) => ReplicaLevel::Full,
            (None, Some(_)) => ReplicaLevel::Meta,
            (None, None) => ReplicaLevel::Probed,
        };
        let create = ReplicaPlacement {
            collection: collection_id,
            handle: handle
                .clone()
                .unwrap_or_else(|| ReplicaHandle(format!("queue-{row_id}"))),
            link_id: Some(link),
            object: object.clone(),
            level,
            meta: meta.clone(),
            // NOTE: a queue producer is not a connector, so it derives no
            // sort key; the sync that pushes this create resolves one.
            sort_key: ReplicaSortKey::default(),
            flags: flags.clone(),
            status: ReplicaStatus::Created,
            conflict_revision: None,
            base: None,
            origin: None,
            ambiguous_handles: Vec::new(),
        };
        return Ok(Ok(vec![ReplicaWriteOp::UpsertPlacement(create)]));
    }

    // Every other kind reads an existing item, addressed by `seq`.
    let (seq, removes) = match action {
        PimdirAction::SetFlags { seq, .. }
        | PimdirAction::Move { seq, .. }
        | PimdirAction::Copy { seq, .. }
        | PimdirAction::Update { seq, .. } => (*seq, false),
        PimdirAction::Remove { seq } => (*seq, true),
        PimdirAction::Add { .. } => unreachable!("add staged above"),
        PimdirAction::Unknown { .. } => unreachable!("unknown kinds are skipped, never staged"),
    };
    let item = tx
        .query_row(
            sql::GET_ITEM,
            named_params! { ":collection": collection, ":seq": seq },
            read_item_from_row,
        )
        .optional()?;
    let Some(item) = item else {
        // NOTE: a remove of an already-absent item is success, not an error
        // (spec §15.3); anything else addressing a gone item parks.
        return if removes {
            Ok(Ok(Vec::new()))
        } else {
            Ok(Err(format!("unknown seq: {seq}")))
        };
    };

    // NOTE: the binding's own primary key answers this, so it is a seek. It
    // used to project the whole collection to read one handle out of it, once
    // per drained action.
    let handle = tx
        .query_row(
            sql::HANDLE_FOR_LINK,
            named_params! {
                ":collection": collection,
                ":link_id": item.link_id.0,
                ":source": source.0,
            },
            |r| r.get::<_, String>(0),
        )
        .optional()?;
    let handle = match handle {
        Some(handle) => ReplicaHandle(handle),
        None if removes => return Ok(Ok(Vec::new())),
        None => return Ok(Err(format!("seq {seq} projects no placement"))),
    };

    let mutation = match action {
        PimdirAction::SetFlags { flags, .. } => ReplicaMutation::SetFlags {
            handle,
            flags: flags.clone(),
        },
        PimdirAction::Remove { .. } => ReplicaMutation::Remove(handle),
        PimdirAction::Move { to, .. } => ReplicaMutation::Move {
            handle,
            target: to.clone(),
            placeholder: ReplicaHandle(format!("queue-{row_id}")),
        },
        PimdirAction::Copy { to, .. } => ReplicaMutation::Copy {
            handle,
            target: to.clone(),
            placeholder: ReplicaHandle(format!("queue-{row_id}")),
        },
        PimdirAction::Update { object, meta, .. } => ReplicaMutation::Edit {
            handle,
            // NOTE: the size only rides the StoreObject op, stripped below;
            // the object row was indexed with its real size at enqueue.
            object: ReplicaObject {
                hash: object.clone(),
                size: 0,
            },
            body: Vec::new(),
            meta: meta.clone(),
            // NOTE: as above, a queued update carries no key.
            sort_key: None,
        },
        PimdirAction::Add { .. } => unreachable!("add staged above"),
        PimdirAction::Unknown { .. } => unreachable!("unknown kinds are skipped, never staged"),
    };

    // NOTE: the mutation reads one placement, so the hub is read for the one
    // identity it names rather than for the collection.
    let mut mutate = ReplicaMutate::new(collection_id.clone(), mutation);
    let _ = mutate.resume(None);
    let placements = load_hub_by_link(tx, collection, core::slice::from_ref(&item.link_id.0))?
        .project(&collection_id, source);
    let loaded = ReplicaLoaded {
        placements,
        checkpoint: None,
    };
    match mutate.resume(Some(ReplicaArg::Load(loaded))) {
        ReplicaCoroutineState::Yielded(ReplicaYield::WantsWrite(ops)) => {
            // NOTE: the body already sits in the blob store and its object row
            // was upserted (and pinned) at enqueue; re-storing here would
            // clobber the recorded size with the placeholder.
            let ops = ops
                .into_iter()
                .filter(|op| !matches!(op, ReplicaWriteOp::StoreObject { .. }))
                .collect();
            Ok(Ok(ops))
        }
        ReplicaCoroutineState::Complete(Err(err)) => Ok(Err(err.to_string())),
        state => Ok(Err(format!("unexpected mutate state: {state:?}"))),
    }
}

/// A pimdir store opened as a **producer** (spec §8): a process that is not
/// the owner but legitimately originates mutations (a submission daemon, a
/// server frontend). Its only write is the single enqueue transaction of spec
/// §15.1 — `ensure_collection`, at most one object upsert pinning a body it
/// already wrote durably to the blob directory ([`PimdirBlobs::writer`]), and
/// one queue insert. It never touches items, bindings, sources or the other
/// collections columns, and never creates the schema: it requires a store the
/// owner has already opened at the current schema version.
///
/// This coexists with the store's single-writer serialisation: the guard is
/// the per-transaction `BEGIN IMMEDIATE` plus the busy timeout, and the spec
/// explicitly sanctions the producer's short append transaction beside the
/// owner's batches — the two serialise on the write lock, never interleave.
pub struct PimdirProducer {
    conn: Connection,
    producer: String,
    /// The hash the store names its objects by (spec §5), so a producer
    /// staging a body names it the way the owner will look it up.
    hash: PimdirHashAlgo,
    /// The account collections this producer creates are grouped under
    /// (spec §9.2); `None` in a single-account store.
    account: Option<String>,
}

impl PimdirProducer {
    /// Opens the store rooted at `dir` as producer `producer` (a diagnostic
    /// process name recorded on each row).
    ///
    /// The database must exist at the current schema version: a producer never
    /// creates a store (that is the owner's opening write), so a missing
    /// database errors and a version mismatch is [`PimdirError::Version`].
    pub fn open(dir: impl AsRef<Path>, producer: impl Into<String>) -> Result<Self, PimdirError> {
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(dir.as_ref().join("pimdir.db"), flags)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 30000;",
        )?;

        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version != sql::VERSION {
            return Err(PimdirError::Version { found: version });
        }
        check_version_agreement(&conn, version)?;
        check_rename_cascades(&conn)?;
        let hash = read_hash_algo(&conn, None)?;

        Ok(Self {
            conn,
            producer: producer.into(),
            hash,
            account: None,
        })
    }

    /// The hash this store names its objects by (spec §5).
    pub fn hash_algo(&self) -> PimdirHashAlgo {
        self.hash
    }

    /// The content hash of a whole body, under this store's algorithm: what a
    /// producer names the blob it writes before enqueueing the action that
    /// references it (spec §15.1).
    pub fn hash(&self, bytes: &[u8]) -> ReplicaHash {
        self.hash.hash(bytes)
    }

    /// An incremental hasher for a body streamed into the blob store.
    pub fn hasher(&self) -> PimdirHasher {
        self.hash.hasher()
    }

    /// Binds this producer to an account, so a collection its enqueue creates
    /// is grouped under it (spec §9.2). Mirrors
    /// [`PimdirStore::for_account`].
    pub fn for_account(mut self, account: impl Into<String>) -> Self {
        self.account = Some(account.into());
        self
    }

    /// Appends one action to a collection's queue (spec §15.1), returning the
    /// row's append id.
    ///
    /// Runs exactly the producer transaction, `BEGIN IMMEDIATE` and short:
    /// `ensure_collection`, at most one object upsert when the action's
    /// payload references a body, and one queue insert pinning that body's
    /// hash against garbage collection. When the action carries an object, the
    /// caller has **already written its blob durably** through
    /// [`PimdirBlobs::writer`] (temp → fsync → rename needs no coordination)
    /// and passes the byte size the writer's commit returned; `None` reuses an
    /// object the store already indexes. `created_at` is the caller's RFC 3339
    /// timestamp. When the owner applies the action is the owner's business;
    /// nudging it to run (a signal, a socket) is out of scope.
    pub fn enqueue(
        &mut self,
        collection: &str,
        action: &PimdirAction,
        object_size: Option<u64>,
        created_at: &str,
    ) -> Result<i64, PimdirError> {
        let hash = action.object_hash().cloned();

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        tx.execute(
            sql::ENSURE_COLLECTION,
            named_params! { ":collection": collection, ":account": self.account.as_deref() },
        )?;
        if let (Some(hash), Some(size)) = (&hash, object_size) {
            tx.execute(
                sql::STORE_OBJECT,
                named_params! { ":hash": hash.0, ":size": size as i64 },
            )?;
        }
        tx.execute(
            sql::ENQUEUE_ACTION,
            named_params! {
                ":created_at": created_at,
                ":producer": self.producer,
                ":collection": collection,
                ":action": action.kind(),
                ":payload": codec::action_to_payload(action),
                ":object_hash": hash.as_ref().map(|h| h.0.as_str()),
            },
        )?;
        // NOTE: the incremental pin (+1): the queue row now references the
        // body, so garbage collection never sweeps it between enqueue and
        // apply; the drain releases it as the row is deleted.
        if let Some(hash) = &hash {
            tx.execute(
                sql::ADJUST_REFCOUNT,
                named_params! { ":delta": 1, ":hash": hash.0 },
            )?;
        }
        let id = tx.last_insert_rowid();
        tx.commit().map_err(busy_or_sql)?;
        Ok(id)
    }

    /// The collection's pending (non-parked) actions in append order — the
    /// producer's read-your-writes overlay (spec §15.4): a just-enqueued
    /// action shows here before the owner has applied it.
    pub fn pending_actions(
        &self,
        collection: &str,
    ) -> Result<Vec<PimdirPendingAction>, PimdirError> {
        load_pending_actions(&self.conn, collection)
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
    hash: PimdirHashAlgo,
}

impl PimdirBlobs {
    /// Opens the blob handle for the store rooted at `dir`, naming bodies with
    /// `hash`.
    ///
    /// The algorithm is the store's, not a choice made here: it is what the
    /// files are named by. [`PimdirStore::blobs`] hands one out already bound to
    /// the store it came from, which is how a consumer avoids picking one.
    pub fn open(dir: impl AsRef<Path>, hash: PimdirHashAlgo) -> Self {
        Self {
            root: dir.as_ref().join("objects"),
            hash,
        }
    }

    /// The hash bodies here are named by.
    pub fn hash_algo(&self) -> PimdirHashAlgo {
        self.hash
    }

    /// The content hash of a whole body, under this store's algorithm.
    pub fn hash(&self, bytes: &[u8]) -> ReplicaHash {
        self.hash.hash(bytes)
    }

    /// An incremental hasher, for a body streamed through
    /// [`writer`](Self::writer) rather than held whole in memory.
    pub fn hasher(&self) -> PimdirHasher {
        self.hash.hasher()
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
        if let Some(parent) = path.parent() {
            sync_dir(parent)?;
        }
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

/// Applies a write batch's ops inside the caller's transaction: blob and object
/// writes, checkpoint upserts, and placement ops folded per collection through
/// the hub (absorb, then persist only what changed: diff the loaded hub against
/// the absorbed one — touch just the changed items/bindings — and adjust object
/// refcounts by only the per-hash change in references, never a
/// whole-collection rewrite or a global refcount recompute).
///
/// Shared by the seam's [`write`](ReplicaStorage::write), the rekey write
/// ([`PimdirStore::write_rekeyed`]) and the queue drain
/// ([`PimdirStore::drain_collection`]), so each wraps the same folding in its
/// own transaction shape.
fn apply_ops(
    tx: &Connection,
    blobs: &Path,
    source: &ReplicaSourceId,
    account: Option<&str>,
    residual: &mut HashMap<(ReplicaCollectionId, ReplicaHandle), ReplicaPlacement>,
    ops: Vec<ReplicaWriteOp>,
) -> Result<(), PimdirError> {
    // Placement/drop ops routed to the hub, grouped by collection.
    let mut hub_ops: BTreeMap<String, Vec<ReplicaWriteOp>> = BTreeMap::new();

    for op in ops {
        match op {
            ReplicaWriteOp::StoreObject { object, body } => {
                // NOTE: a byteless op indexes an object the consumer already
                // streamed into the blob store during a fetch (bounded-memory
                // transfer); inline bytes are the buffered path.
                if let Some(body) = body {
                    write_blob(blobs, &object.hash.0, &body)?;
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
                    named_params! { ":collection": collection.0, ":account": account },
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
                    drop_residual(residual, &placement.collection, &placement.handle);
                    hub_ops
                        .entry(placement.collection.0.clone())
                        .or_default()
                        .push(ReplicaWriteOp::UpsertPlacement(placement));
                } else {
                    // NOTE: not yet linked — stage in the residual until a
                    // Meta upgrade resolves its link id.
                    let key = (placement.collection.clone(), placement.handle.clone());
                    residual.insert(key, placement);
                }
            }
            ReplicaWriteOp::DropPlacement {
                collection,
                handle,
                reason,
            } => {
                drop_residual(residual, &collection, &handle);
                hub_ops.entry(collection.0.clone()).or_default().push(
                    ReplicaWriteOp::DropPlacement {
                        collection,
                        handle,
                        reason,
                    },
                );
            }
        }
    }

    for (collection, ops) in hub_ops {
        let links = batch_links(tx, &collection, source, &ops)?;
        let old_hub = load_hub_by_link(tx, &collection, &links)?;
        let mut new_hub = old_hub.clone();
        new_hub.absorb(source, &ops);
        save_hub_diff(tx, &collection, source, account, &old_hub, &new_hub)?;
        adjust_refcounts(tx, &object_refs(&old_hub), &object_refs(&new_hub))?;
    }

    Ok(())
}

/// Deletes the zero-refcount object rows inside the caller's transaction and
/// returns their hashes and sizes; the caller unlinks the blob files **after**
/// the commit, so a crash leaves at worst an orphan blob, never a row without
/// its body. The sizes are what a purge reports as bytes reclaimed.
fn collect_garbage(tx: &Connection) -> Result<Vec<(String, u64)>, rusqlite::Error> {
    let garbage: Vec<(String, u64)> = {
        let mut stmt = tx.prepare(sql::LIST_GARBAGE_SIZED)?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?.max(0) as u64))
        })?;
        let mut objects = Vec::new();
        for row in rows {
            objects.push(row?);
        }
        objects
    };
    tx.execute(sql::DELETE_GARBAGE_OBJECTS, [])?;
    Ok(garbage)
}

/// Creates the schema in a fresh database (spec §6), advancing `user_version`
/// and seeding `store_meta.version` in agreement (spec §4.2) inside one
/// transaction. A store stamped with a `user_version` higher than
/// [`sql::VERSION`] is refused: the spec is a draft with a single schema
/// version, so such a store is recreated, never migrated.
fn init_schema(conn: &mut Connection, hash: PimdirHashAlgo) -> Result<(), PimdirError> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > sql::VERSION {
        return Err(PimdirError::Version { found: version });
    }
    if version == sql::VERSION {
        check_version_agreement(conn, version)?;
        check_rename_cascades(conn)?;
        return reconcile_draft_shape(conn);
    }

    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(busy_or_sql)?;
    tx.execute_batch(sql::MIGRATION_0001)?;
    // NOTE: the script creates `store_meta`; seed its one row here, since the
    // canonical script is pure DDL. The timestamp is SQLite's own, in the
    // RFC 3339 form the column is declared to hold and the retirement clock
    // already writes, which also keeps the crate free of a clock: reading
    // one and formatting it by hand is what had this column holding epoch
    // milliseconds, and the empty string when the clock predates the epoch.
    tx.execute(
        "INSERT OR IGNORE INTO store_meta(id, version, hash_algo, created_at) \
         VALUES(1, ?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![sql::VERSION, hash.as_str()],
    )?;
    tx.pragma_update(None, "user_version", sql::VERSION)?;
    tx.commit().map_err(busy_or_sql)?;

    Ok(())
}

/// Refuses a store whose foreign keys predate the `ON UPDATE CASCADE` every
/// key onto a renamed row now carries (spec §14).
///
/// This is the half of the draft allowance (spec §6) that reconciliation
/// cannot reach: a column can be added in place, a foreign-key action cannot,
/// short of rebuilding every table that carries one. §6's other branch is to
/// refuse the store and tell the operator to recreate it, which costs a resync
/// of what is by design a derived cache (spec §1) and is what this does.
///
/// Without the cascade a rename is refused by SQLite one dependent row down,
/// so such a store can never follow a server-side collection rename or an
/// account rename; catching it on open says so once, rather than at the moment
/// a rename fails.
fn check_rename_cascades(conn: &Connection) -> Result<(), PimdirError> {
    /// The tables whose foreign key onto a renamable parent must cascade, with
    /// that parent. `bindings` is the second one the rename needs: it hangs off
    /// `items(collection, link_id)`, which the first cascade updates.
    const CASCADING: [(&str, &str); 5] = [
        ("collections", "collections"),
        ("sources", "collections"),
        ("items", "collections"),
        ("bindings", "items"),
        ("queue", "collections"),
    ];

    for (table, parent) in CASCADING {
        let mut stmt = conn.prepare(&format!(
            "SELECT on_update FROM pragma_foreign_key_list('{table}') WHERE \"table\" = '{parent}'"
        ))?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let on_update: String = row.get(0)?;
            if on_update != "CASCADE" {
                return Err(PimdirError::Unreconcilable { table });
            }
        }
    }

    Ok(())
}

/// Adds columns folded into version 1 after a store was already created at
/// version 1 (spec §6, the `draft` allowance).
///
/// While the spec is a draft, version 1 is not frozen: a schema change may be
/// folded into `0001_init.sql` rather than added as version 2. The cost is that
/// a store written by an earlier draft is not *detectably* out of date — its
/// `user_version` already matches, so the runner would do nothing and the
/// missing column would surface much later as a query error. §6 requires an
/// implementation to reconcile the shape on open or refuse the store outright;
/// this reconciles.
///
/// `ALTER TABLE … ADD COLUMN` is cheap (a metadata-only rewrite for a column
/// with a constant default), and guarding on `PRAGMA table_info` makes it a
/// no-op for a current store, which is every store after the first open. Only
/// columns that are nullable or carry a default can be folded in this way.
///
/// This disappears when the spec leaves `draft`; from the first frozen version
/// onwards, a shape change is an ordinary numbered migration.
fn reconcile_draft_shape(conn: &mut Connection) -> Result<(), PimdirError> {
    /// Columns folded into version 1 after it was first published, as
    /// `(table, column, declaration)`. Each must be nullable or carry a
    /// constant default, or it could not be added to a populated table.
    const FOLDED_IN: [(&str, &str, &str); 8] = [
        ("bindings", "conflicted", "INTEGER NOT NULL DEFAULT 0"),
        ("bindings", "conflict_revision", "TEXT"),
        ("items", "retained_at", "TEXT"),
        ("items", "retained_by", "TEXT"),
        ("collections", "account", "TEXT"),
        ("items", "sort_key", "TEXT NOT NULL DEFAULT ''"),
        ("bindings", "ambiguous_handles", "TEXT"),
        ("bindings", "base_present", "INTEGER NOT NULL DEFAULT 0"),
    ];

    let mut missing = Vec::new();
    for (table, column, decl) in FOLDED_IN {
        if !has_column(conn, table, column)? {
            missing.push((table, column, decl));
        }
    }

    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(busy_or_sql)?;
    for (table, column, decl) in missing {
        tx.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"))?;
    }
    // NOTE: unconditionally, unlike the columns. An index over a folded-in
    // column has to be created with it, but most of these index columns that
    // were always there: what changed is that a statement now needs them. A
    // store that kept the old plans would keep scanning where the schema says
    // it seeks, silently and for good.
    tx.execute_batch(sql::ENSURE_INDEXES)?;
    tx.commit().map_err(busy_or_sql)?;
    Ok(())
}

/// The algorithm the store records, checked against the one the caller declared.
///
/// A store names every blob by its hash, so a handle computing a different one
/// writes bodies no reader of that store finds and dedups against nothing. The
/// failure is silent by nature, which is why it is caught on open rather than
/// left to surface as a cache that never hits.
fn read_hash_algo(
    conn: &Connection,
    declared: Option<PimdirHashAlgo>,
) -> Result<PimdirHashAlgo, PimdirError> {
    let stored: Option<String> = conn
        .query_row("SELECT hash_algo FROM store_meta WHERE id = 1", [], |row| {
            row.get(0)
        })
        .optional()?;

    let Some(stored) = stored else {
        return Ok(declared.unwrap_or_default());
    };
    let Some(algo) = PimdirHashAlgo::parse(&stored) else {
        return Err(PimdirError::HashAlgo {
            found: stored,
            declared: declared.map(|a| a.as_str()),
        });
    };
    match declared {
        Some(declared) if declared != algo => Err(PimdirError::HashAlgo {
            found: stored,
            declared: Some(declared.as_str()),
        }),
        _ => Ok(algo),
    }
}

/// The two schema stamps a store carries, which spec §4.2 requires to agree:
/// `PRAGMA user_version` and `store_meta.version`. A store where they differ is
/// corrupt, so it is refused rather than read at the version one of them names.
///
/// A store whose `store_meta` row is absent is left alone: the row is seeded by
/// whoever created the schema, and refusing here would turn a missing stamp
/// into an unopenable store the crate could otherwise repair.
fn check_version_agreement(conn: &Connection, user_version: i64) -> Result<(), PimdirError> {
    let stamped: Option<i64> = conn
        .query_row("SELECT version FROM store_meta WHERE id = 1", [], |row| {
            row.get(0)
        })
        .optional()?;

    match stamped {
        Some(store_meta) if store_meta != user_version => Err(PimdirError::VersionMismatch {
            user_version,
            store_meta,
        }),
        _ => Ok(()),
    }
}

/// Whether `table` already has `column`, via `PRAGMA table_info`.
fn has_column(conn: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Removes any residual placement matching `(collection, handle)`.
fn drop_residual(
    residual: &mut HashMap<(ReplicaCollectionId, ReplicaHandle), ReplicaPlacement>,
    collection: &ReplicaCollectionId,
    handle: &ReplicaHandle,
) {
    residual.remove(&(collection.clone(), handle.clone()));
}

/// Loads a collection's [`ReplicaHub`] (items + per-source bindings + policy).
fn load_hub(conn: &Connection, collection: &str) -> rusqlite::Result<ReplicaHub> {
    read_hub(conn, collection, None)
}

/// The link ids one write batch touches: the ones its upserts carry, plus the
/// ones its drops resolve to, since a drop names a handle and the shared item is
/// keyed by link id.
///
/// A handle no binding holds resolves to nothing and is simply left out: there
/// is no item to fold the drop into.
fn batch_links(
    conn: &Connection,
    collection: &str,
    source: &ReplicaSourceId,
    ops: &[ReplicaWriteOp],
) -> rusqlite::Result<Vec<String>> {
    let mut links: BTreeSet<String> = BTreeSet::new();

    for op in ops {
        match op {
            ReplicaWriteOp::UpsertPlacement(placement) => {
                if let Some(link) = &placement.link_id {
                    links.insert(link.0.clone());
                }
            }
            ReplicaWriteOp::DropPlacement { handle, .. } => {
                let link = conn
                    .query_row(
                        sql::LINK_FOR_HANDLE,
                        named_params! {
                            ":collection": collection,
                            ":source": source.0,
                            ":handle": handle.0,
                        },
                        |r| r.get::<_, String>(0),
                    )
                    .optional()?;
                links.extend(link);
            }
            _ => {}
        }
    }

    Ok(links.into_iter().collect())
}

/// The hub narrowed to `links`, which is what a write folds its batch into.
///
/// The batch only ever produces writes for the items it names, so the rest of
/// the collection would be read, cloned and diffed to conclude that nothing
/// changed: the cost of one flag on one message would be the size of the
/// mailbox. Both sides of the diff are narrowed the same way, so every
/// comparison the persistence step makes, and every object reference the
/// refcount step counts, sees exactly what it would have seen in full.
fn load_hub_by_link(
    conn: &Connection,
    collection: &str,
    links: &[String],
) -> rusqlite::Result<ReplicaHub> {
    read_hub(conn, collection, Some(links))
}

/// The shared reader behind [`load_hub`] and [`load_hub_by_link`]: `None` reads
/// the whole collection, `Some` only the named link ids.
fn read_hub(
    conn: &Connection,
    collection: &str,
    links: Option<&[String]>,
) -> rusqlite::Result<ReplicaHub> {
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

    // NOTE: the scoped statements name a `:links` the unscoped ones do not,
    // and a bound parameter a statement never declared is an error, so each
    // shape is prepared and bound on its own.
    match links {
        Some(links) => {
            let scope = serde_json::to_string(links).unwrap_or_else(|_| "[]".into());
            let params = named_params! { ":collection": collection, ":links": scope };

            let mut items = conn.prepare(sql::LOAD_ITEMS_BY_LINK)?;
            read_hub_items(&mut hub, items.query_map(params, item_from_row)?)?;
            let mut bindings = conn.prepare(sql::LOAD_BINDINGS_BY_LINK)?;
            read_hub_bindings(&mut hub, bindings.query_map(params, binding_from_row)?)?;
        }
        None => {
            let params = named_params! { ":collection": collection };

            let mut items = conn.prepare(sql::LOAD_ITEMS)?;
            read_hub_items(&mut hub, items.query_map(params, item_from_row)?)?;
            let mut bindings = conn.prepare(sql::LOAD_BINDINGS)?;
            read_hub_bindings(&mut hub, bindings.query_map(params, binding_from_row)?)?;
        }
    }

    Ok(hub)
}

fn read_hub_items<F>(
    hub: &mut ReplicaHub,
    rows: rusqlite::MappedRows<'_, F>,
) -> rusqlite::Result<()>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<(ReplicaLinkId, ReplicaHubItem)>,
{
    for row in rows {
        let (link, item) = row?;
        hub.items.insert(link, item);
    }
    Ok(())
}

fn read_hub_bindings<F>(
    hub: &mut ReplicaHub,
    rows: rusqlite::MappedRows<'_, F>,
) -> rusqlite::Result<()>
where
    F: FnMut(
        &rusqlite::Row<'_>,
    ) -> rusqlite::Result<(ReplicaLinkId, ReplicaSourceId, ReplicaSourceBinding)>,
{
    for row in rows {
        let (link, source, binding) = row?;
        if let Some(item) = hub.items.get_mut(&link) {
            item.sources.insert(source, binding);
        }
    }
    Ok(())
}

/// Persists the change from `old` to `new` for a collection's hub by diffing
/// the two in memory and issuing only the item and binding inserts, updates and
/// deletes that actually differ, never a whole-collection delete-and-reinsert.
///
/// Paired with a batch-scoped read ([`load_hub_by_link`]) that makes both
/// halves of a write proportional to the batch rather than to the collection:
/// the rows it reads as well as the rows it writes. An item no source holds any
/// more is retained rather than deleted, `source` naming the side whose removal
/// retired it.
fn save_hub_diff(
    conn: &Connection,
    collection: &str,
    source: &ReplicaSourceId,
    account: Option<&str>,
    old: &ReplicaHub,
    new: &ReplicaHub,
) -> rusqlite::Result<()> {
    conn.execute(
        sql::ENSURE_COLLECTION,
        named_params! { ":collection": collection, ":account": account },
    )?;
    if old.conflict != new.conflict {
        conn.execute(
            sql::SET_CONFLICT,
            named_params! { ":collection": collection, ":conflict": conflict_to_str(new.conflict) },
        )?;
    }

    // Items gone in `new`: no source holds them any more, so they are retained
    // (soft-deleted), never deleted: a store loses an item only to a purge.
    // The bindings go with the sources that held them; the row stays, hidden
    // from `LOAD_ITEMS` so no later sync, delta or full, re-derives against it.
    for (link, item) in &old.items {
        if new.items.contains_key(link) {
            continue;
        }
        conn.execute(
            sql::RETAIN_ITEM,
            named_params! { ":collection": collection, ":link_id": link.0, ":source": source.0 },
        )?;
        conn.execute(
            sql::DELETE_ITEM_BINDINGS,
            named_params! { ":collection": collection, ":link_id": link.0 },
        )?;
        // NOTE: the caller's refcount diff is about to release this item's
        // object references as it leaves the hub, but the row survives and
        // still points at them. Pin them back, exactly as a queue row pins a
        // queued body, so garbage collection cannot sweep a retained body.
        // Revive and purge release the pin.
        for hash in [item.object.as_ref(), item.conflict_object.as_ref()]
            .into_iter()
            .flatten()
        {
            conn.execute(
                sql::ADJUST_REFCOUNT,
                named_params! { ":delta": 1, ":hash": hash.0 },
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
///
/// Every column `UPDATE_ITEM` writes has to be here: one left out is a
/// column that can never change again, since the diff reports the row
/// unchanged and no statement is issued for it.
fn item_columns_eq(a: &ReplicaHubItem, b: &ReplicaHubItem) -> bool {
    a.flags == b.flags
        && a.object == b.object
        && a.meta == b.meta
        && a.sort_key == b.sort_key
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
    // A retained row may still hold this primary key: the item is back, either
    // resurrected on a source or restored by a client `add` over the values the
    // row still carries. Revive it in place instead of colliding, keeping its
    // `seq` (a message keeps one public id for life, and ids are never reused).
    if revive_item(conn, collection, link, item)? {
        return Ok(());
    }

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
            ":sort_key": item.sort_key.0.as_str(),
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

/// Revives the retained row holding `(collection, link)`, if there is one: it
/// stops being retained (spec §11), adopts the incoming content through the
/// ordinary item update and binds the sources. Returns whether a row was
/// revived.
///
/// The retention pin the retire took is released here; the caller's refcount
/// diff takes the live reference for the adopted content in the same
/// transaction, so a body kept only by the retained row is never sweepable in
/// between.
fn revive_item(
    conn: &Connection,
    collection: &str,
    link: &ReplicaLinkId,
    item: &ReplicaHubItem,
) -> rusqlite::Result<bool> {
    let pinned: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            sql::RETAINED_ITEM,
            named_params! { ":collection": collection, ":link_id": link.0 },
            |row| Ok((row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((object, conflict_object)) = pinned else {
        return Ok(false);
    };

    conn.execute(
        sql::REVIVE_ITEM,
        named_params! { ":collection": collection, ":link_id": link.0 },
    )?;
    update_item(conn, collection, link, item)?;
    for hash in [object, conflict_object].into_iter().flatten() {
        conn.execute(
            sql::ADJUST_REFCOUNT,
            named_params! { ":delta": -1, ":hash": hash },
        )?;
    }
    for (source, binding) in &item.sources {
        insert_binding(conn, collection, link, source, binding)?;
    }
    Ok(true)
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
            ":sort_key": item.sort_key.0.as_str(),
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
                // NOTE: a binding pins one handle, and repointing it is how
                // the fact that a source holds an identity twice used to be
                // destroyed, silently, at this write: no later rule could act
                // on it because the evidence was already gone. The bound
                // handle stays and the incoming one is recorded instead,
                // which freezes the item. The engine no longer produces such
                // an upsert, so this is the floor under it: a store written
                // by an older one, or a consumer staging its own writes.
                let mut binding = binding.clone();
                if binding.handle != prev.handle {
                    let incoming = mem::replace(&mut binding.handle, prev.handle.clone());
                    if !binding.ambiguous_handles.contains(&incoming) {
                        binding.ambiguous_handles.push(incoming);
                    }
                }
                update_binding(conn, collection, link, source, &binding)?
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
            ":base_present": binding.base.is_some() as i64,
            ":conflicted": binding.conflicted as i64,
            ":conflict_revision": binding.conflicted.then_some(binding.conflict_revision.as_deref()).flatten(),
            ":ambiguous_handles": codec::handles_to_json(&binding.ambiguous_handles),
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
            ":base_flags": binding.base.as_ref().map(|b| codec::flags_to_json(&b.flags)),
            ":base_object": binding.base.as_ref().and_then(|b| b.object.as_ref()).map(|o| o.0.as_str()),
            ":base_revision": binding.base.as_ref().and_then(|b| b.revision.as_deref()),
            ":base_present": binding.base.is_some() as i64,
            ":conflicted": binding.conflicted as i64,
            ":conflict_revision": binding.conflicted.then_some(binding.conflict_revision.as_deref()).flatten(),
            ":ambiguous_handles": codec::handles_to_json(&binding.ambiguous_handles),
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
    let sort_key: String = row.get(5)?;
    let level: i64 = row.get(6)?;

    Ok(PimdirItem {
        seq,
        link_id: ReplicaLinkId(link),
        flags: codec::flags_from_json(flags.as_deref()),
        meta: meta.map(ReplicaMeta),
        sort_key,
        object: object.map(ReplicaHash),
        level: codec::level_from_int(level),
    })
}

fn item_from_row(row: &Row) -> rusqlite::Result<(ReplicaLinkId, ReplicaHubItem)> {
    let link: String = row.get(0)?;
    let flags: Option<String> = row.get(1)?;
    let object: Option<String> = row.get(2)?;
    let meta: Option<String> = row.get(3)?;
    let sort_key: String = row.get(4)?;
    let level: i64 = row.get(5)?;
    let deleted: i64 = row.get(6)?;
    let conflicted: i64 = row.get(7)?;
    let conflict_object: Option<String> = row.get(8)?;

    Ok((
        ReplicaLinkId(link),
        ReplicaHubItem {
            flags: codec::flags_from_json(flags.as_deref()),
            object: object.map(ReplicaHash),
            meta: meta.map(ReplicaMeta),
            sort_key: ReplicaSortKey(sort_key),
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
    let base_present: i64 = row.get(6)?;
    let conflicted: i64 = row.get(7)?;
    let conflict_revision: Option<String> = row.get(8)?;
    let ambiguous_handles: Option<String> = row.get(9)?;

    // NOTE: either witness. The column is the fact, and a base of no revision,
    // no body and markers nobody has read is a real agreement its three value
    // columns cannot express: reading presence off them alone has such a
    // placement come back as never-agreed, so the sync re-derives the same push
    // on every run. The value columns stay a witness for a row written before
    // the column existed, where they are the only evidence there is.
    let base = if base_present != 0
        || base_flags.is_some()
        || base_object.is_some()
        || base_revision.is_some()
    {
        Some(ReplicaBase {
            flags: codec::flags_from_json(base_flags.as_deref()),
            revision: base_revision,
            object: base_object.map(ReplicaHash),
        })
    } else {
        None
    };

    let conflicted = conflicted != 0;
    Ok((
        ReplicaLinkId(link),
        ReplicaSourceId(source),
        ReplicaSourceBinding {
            handle: ReplicaHandle(handle),
            base,
            conflicted,
            // Spec §13: the revision is meaningful only while conflicted, so a
            // resolved binding cannot hand a stale one to the next sync even if
            // the column somehow still holds one.
            conflict_revision: conflicted.then_some(conflict_revision).flatten(),
            ambiguous_handles: codec::handles_from_json(ambiguous_handles.as_deref()),
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
    fs::rename(&tmp, &path)?;
    sync_dir(parent)
}

/// Flushes a directory entry, so a rename into it survives a power loss.
///
/// Syncing the file makes its bytes durable and says nothing about the name
/// that reaches them. The database commit is durable, so without this a crash
/// can leave a committed row pointing at a body that never arrived: the one
/// asymmetry the write order exists to prevent, since the reverse leaves at
/// worst an orphan blob.
fn sync_dir(dir: &Path) -> io::Result<()> {
    fs::File::open(dir)?.sync_all()
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
    /// The SQLite index refused a statement, or the connection itself failed.
    Sql(rusqlite::Error),
    /// The blob directory refused a read, a write or a rename.
    Io(io::Error),
    /// JSON encoding failed at the storage seam (the link id array a lookup
    /// hands to SQLite); a malformed queue payload reports as `Action`.
    Json(serde_json::Error),
    /// A queue action payload is malformed or unsupported (spec §15.3).
    Action(PimdirActionError),
    /// The store's schema version is not one this opener can service: newer
    /// than the crate for an owner, or not yet created for a producer (which
    /// never creates the schema; the owner must open first).
    Version {
        /// The store's `user_version`.
        found: i64,
    },
    /// The store's two schema stamps disagree, which spec §4.2 defines as
    /// corruption: `PRAGMA user_version` and `store_meta.version` mirror one
    /// another, so a store where they differ was half-written by something and
    /// is refused rather than read.
    VersionMismatch {
        /// The store's `PRAGMA user_version`.
        user_version: i64,
        /// The version its `store_meta` row records.
        store_meta: i64,
    },
    /// The store was created by a draft whose foreign keys lack the
    /// `ON UPDATE CASCADE` a rename depends on (spec §14), which no
    /// `ALTER TABLE` can add. Spec §6's other branch applies: the operator
    /// recreates the store, which is a resync of a derived cache.
    Unreconcilable {
        /// The first table found without the cascade.
        table: &'static str,
    },
    /// The store's `store_meta.hash_algo` is not one this crate computes, or
    /// not the one the caller declared. Either way the handle would name bodies
    /// the store does not use, so it is refused instead (spec §5).
    HashAlgo {
        /// The algorithm the store records.
        found: String,
        /// The algorithm the caller declared, when it declared one.
        declared: Option<&'static str>,
    },
    /// Another writer holds the store's single write lock (§8); the caller
    /// should retry once the other writer (a sync, another client) is done.
    Busy,
}

impl fmt::Display for PimdirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PimdirError::Sql(err) => write!(f, "pimdir SQL error: {err}"),
            PimdirError::Io(err) => write!(f, "pimdir I/O error: {err}"),
            PimdirError::Json(err) => write!(f, "pimdir JSON error: {err}"),
            PimdirError::Action(err) => write!(f, "pimdir action error: {err}"),
            PimdirError::Version { found } => write!(
                f,
                "pimdir store schema version {found} is unsupported (this crate services version {})",
                sql::VERSION
            ),
            PimdirError::VersionMismatch {
                user_version,
                store_meta,
            } => write!(
                f,
                "pimdir store is corrupt: PRAGMA user_version is {user_version} but store_meta records {store_meta}"
            ),
            PimdirError::Unreconcilable { table } => write!(
                f,
                "pimdir store predates the ON UPDATE CASCADE on `{table}`, which cannot be added in place: delete the store and let it resync"
            ),
            PimdirError::HashAlgo {
                found,
                declared: Some(declared),
            } => write!(
                f,
                "pimdir store names its objects with `{found}`, not the `{declared}` this handle declared"
            ),
            PimdirError::HashAlgo {
                found,
                declared: None,
            } => write!(
                f,
                "pimdir store names its objects with `{found}`, which this crate does not compute"
            ),
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

impl From<PimdirActionError> for PimdirError {
    fn from(err: PimdirActionError) -> Self {
        PimdirError::Action(err)
    }
}
