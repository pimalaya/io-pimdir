//! [`PimdirStore`]: the std store that services [`io_replica`]'s storage seam.
//!
//! It persists a [`ReplicaHub`] per collection, one shared item plus a
//! base per source, and splits by whether an operation has a side at all:
//! [`PimdirStore`] is the store itself (the client reads, retention, the
//! queue), and [`PimdirSourceStore`], which [`for_source`] yields,
//! services [`ReplicaStorage`] for one source. A single-source store is
//! the N=1 case. A freshly probed placement of a handle nothing binds
//! has no link id to key an item on yet, so it is held in memory as a
//! residual until a `Meta` upgrade resolves it.
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
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::Arc,
};

use io_replica::{
    change::{ReplicaDropReason, ReplicaWriteOp},
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
    Connection, ErrorCode, OpenFlags, OptionalExtension, Params, Row, TransactionBehavior,
    named_params, params, types::ToSql,
};

use crate::{
    client::{lock::PimdirLock, reader::PimdirReader},
    codec::{self, PimdirAction, PimdirActionError},
    hash::{PimdirHashAlgo, PimdirHasher},
    sql,
};

pub mod diagnostics;
pub mod reader;

mod lock;

/// A pimdir store held as its owner: the write surface, over the read
/// surface every role shares.
///
/// It carries what only an owner may do, none of which consults a
/// source: retention and purge, the sweep and its repairs, and the queue
/// rows a drain or a cancellation removes. The sync seam does consult
/// one, and lives on [`PimdirSourceStore`], which
/// [`for_source`](Self::for_source) yields. Reading is not an owner's
/// privilege, so the reads live on [`PimdirReader`] and this handle
/// dereferences to one.
pub struct PimdirStore {
    reader: PimdirReader,
    /// The store's exclusive owner lock (spec §8), held for this handle's
    /// lifetime; `None` on a handle opened through the deprecated
    /// read-only constructor. Several handles of one process share one
    /// lock.
    _lock: Option<Arc<PimdirLock>>,
    /// The account every collection this handle creates belongs to (spec §9.2);
    /// `None` in a single-account store. Set with
    /// [`for_account`](PimdirStore::for_account).
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

/// A pimdir store acting as one source (`"left"`, `"right"`, `"phone"`, …):
/// the sync seam, where every operation means "as this side".
///
/// The underlying database and blobs are shared: several sources of one
/// store are several handles over the same files. Dereferences to the
/// [`PimdirStore`] it was made from, so the source-less surface stays
/// reachable through it.
pub struct PimdirSourceStore {
    store: PimdirStore,
    source: ReplicaSourceId,
    /// Probed placements of handles no binding holds, awaiting the `Meta`
    /// upgrade that gives them a link id; empty at rest between syncs. A
    /// probe of a bound handle is not one of these: it keys onto the item
    /// that binding names.
    ///
    /// Keyed rather than listed: a first sync probes a whole collection
    /// before linking any of it, so the residual grows to the collection
    /// size while every insertion, drop and lookup searches it.
    residual: HashMap<(ReplicaCollectionId, ReplicaHandle), ReplicaPlacement>,
}

/// A collection as seen by a client read (`list_collections`): its
/// identity and presentation, kind-agnostic. The sync bindings and
/// per-source state are not exposed here: a reader observes the shared
/// truth only.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirCollection {
    /// The stable collection id (the mailbox name for a mail store).
    pub id: String,
    /// The account this collection is grouped under (spec §9.2), `None`
    /// in a single-account store. It groups and nothing more: no
    /// identifier is scoped by it.
    pub account: Option<String>,
    /// The declared IANA media type (`message/rfc822`, `text/vcard`, …),
    /// or the empty string when a sync created the collection before a
    /// kind was set.
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
    /// The handle-space epoch (spec §12): starts at 1, bumped by the
    /// owner only on a rekey, so a frontend derives epoch-dependent
    /// protocol values (an IMAP UIDVALIDITY) from the store alone.
    pub generation: i64,
}

/// Where one identity or one body sits, as the multiplicity reads report
/// it (spec §9.2): one row per live placement, carrying the collection
/// and account it occurs in.
///
/// A fact, not a verdict. The same vCard `UID` in two accounts' address
/// books is two of these; whether that is one person shown twice or two
/// people is the consumer's call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirPlacement {
    /// The collection the placement sits in.
    pub collection: String,
    /// The account that collection is grouped under, `None` when ungrouped.
    pub account: Option<String>,
    /// The item's public id, shared by every placement of one link id.
    pub seq: i64,
    /// The cross-collection identity.
    pub link_id: ReplicaLinkId,
    /// The body this placement points at, absent until hydrated.
    pub object: Option<ReplicaHash>,
    /// The placement's flag set.
    pub flags: ReplicaFlags,
    /// The detail tier the item is hydrated to.
    pub level: ReplicaLevel,
}

/// One binding whose own sync is stuck on an unresolved content conflict
/// (spec §13), as the conflict listing reports it: what the binding is,
/// and the three bodies a resolver merges.
///
/// The whole divergence, off one row. Base is what the two sides last
/// agreed on, `object` is the local side, `conflict_object` is the remote
/// side at `conflict_revision`, and a resolver reading all three from the
/// store needs no credentials and no round trip.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirConflict {
    /// The collection the conflicted binding sits in.
    pub collection: String,
    /// The item's cross-source identity.
    pub link_id: ReplicaLinkId,
    /// The source that diverged from its own remote. One source can be
    /// conflicted while another holding the same item is in sync, which
    /// is why a conflict is named by this and not by the item alone.
    pub source: ReplicaSourceId,
    /// The item's handle on that source, what a resolver pushes back to.
    pub handle: ReplicaHandle,
    /// The remote revision observed when the divergence was recorded;
    /// `None` when the remote reports none. A resolution computed
    /// against it is stale once it moves.
    pub conflict_revision: Option<String>,
    /// The body the last sync agreed on, the merge's common ancestor;
    /// `None` when the base carried no body.
    pub base_object: Option<ReplicaHash>,
    /// The local side of the divergence, the item's own body.
    pub object: Option<ReplicaHash>,
    /// The remote side at `conflict_revision`; `None` until the upgrade
    /// pass supplies it, which is a conflict that is visible and listable
    /// and not yet resolvable.
    pub conflict_object: Option<ReplicaHash>,
}

/// Maps a `LIST_CONFLICTED_BINDINGS`-shaped row.
fn conflict_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<PimdirConflict> {
    Ok(PimdirConflict {
        collection: r.get(0)?,
        link_id: ReplicaLinkId(r.get(1)?),
        source: ReplicaSourceId(r.get(2)?),
        handle: ReplicaHandle(r.get(3)?),
        conflict_revision: r.get(4)?,
        base_object: r.get::<_, Option<String>>(5)?.map(ReplicaHash),
        object: r.get::<_, Option<String>>(6)?.map(ReplicaHash),
        conflict_object: r.get::<_, Option<String>>(7)?.map(ReplicaHash),
    })
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

/// One live item as seen by a client read (`list_items`/`get_item`): the
/// shared truth a domain projects (an envelope, a vCard, an event),
/// kind-agnostic. The `meta` is the raw stored summary, parsed by the
/// reader against its own schema, and the `level` makes the read
/// availability-aware: below `Full` the body is not local.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirItem {
    /// The message's public id (`items.seq`): a small, stable,
    /// store-global integer, the same across every mailbox the message is
    /// filed in, that a consumer shows and passes back instead of the
    /// long internal `link_id`.
    pub seq: i64,
    /// The cross-source link id (`Message-ID` for mail, UID for a vCard, …).
    /// Internal: a consumer keys reads and edits by `seq`, not this.
    pub link_id: ReplicaLinkId,
    /// The item's flag set.
    pub flags: ReplicaFlags,
    /// The raw per-domain summary blob, verbatim; `None` when never projected.
    pub meta: Option<ReplicaMeta>,
    /// The kind's ordering key (spec §9.3): a normalised RFC 3339 instant
    /// for mail and calendars, a normalised display name for contacts.
    /// Empty means unknown, which sorts before every real key ascending
    /// and after every one descending.
    pub sort_key: String,
    /// The content-addressed body hash; `None` until a `Full` hydrate.
    pub object: Option<ReplicaHash>,
    /// The detail tier the item is hydrated to.
    pub level: ReplicaLevel,
    /// What retention holds about the row, `None` while it is live. The
    /// trash view is the only read that fills it.
    pub retention: Option<PimdirRetention>,
}

/// What retention holds about an item no source binds any more (spec §11), on
/// the row the trash view reads (`list_retained`).
///
/// Only that read fills it: a live item carries `None`, and the two reads
/// are otherwise the same row, which is why they are the same type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirRetention {
    /// The RFC 3339 instant the last binding vanished, not when a server
    /// deleted the item, which is unknowable. A revive clears it, so
    /// restore-then-redelete restarts the purge clock.
    pub at: String,
    /// The source whose removal retired the item; diagnostic only.
    pub by: Option<String>,
    /// The body's size in bytes, `None` alongside an absent `object`:
    /// what lets a caller price a purge without a second query.
    pub size: Option<u64>,
}

/// What a purge retired.
///
/// Rows, not bytes: a purge releases the references a retained item held
/// and nothing more. The bodies they kept are reclaimed by the collector,
/// which is what reports the bytes ([`PimdirStore::collect_garbage`]).
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
    /// Blob files unlinked: those rows' bodies and the orphans a crash
    /// left, together.
    pub blobs: usize,
    /// The bytes those files freed.
    pub bytes: u64,
}

/// One pending (non-parked) queue row, in append order (spec §15.4):
/// what a frontend overlays on its item projection for read-your-writes,
/// and what the owner's drain applies.
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

/// One parked queue row: an action the owner judged permanently
/// unappliable, recorded and skipped instead of blocking its collection's
/// queue. Left for operators, never silently deleted (spec §15.2). The
/// payload stays raw, since being undecodable may be why it parked.
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

/// What a [`drain_collection`](PimdirSourceStore::drain_collection) pass did.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PimdirDrainReport {
    /// Actions applied to the store and deleted from the queue.
    pub applied: usize,
    /// Actions parked with an error, left queryable.
    pub parked: usize,
    /// Actions this owner could not perform, left pending for one that
    /// can (spec §15.2). Not a failure: parking would claim the action is
    /// permanently unappliable, which is a different statement.
    pub skipped: usize,
}

impl PimdirStore {
    /// Opens (creating if absent) the store rooted at `dir`, as its owner.
    ///
    /// The handle takes the store's exclusive advisory lock (spec §8) and
    /// holds it until it drops, so a store has one owner process at a
    /// time; one already owned elsewhere is [`PimdirError::Owned`]
    /// immediately, never a wait. Several handles of one process share
    /// that lock: one per source, or one per account, is still one owner.
    ///
    /// A fresh database is created at the current schema version. A store
    /// stamped with a higher `user_version` than this crate services is
    /// refused with [`PimdirError::Version`] rather than half-read: the
    /// spec is a draft, so such a store is recreated, never migrated.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, PimdirError> {
        Self::open_with_hash(dir, None)
    }

    /// Opens (creating if absent) the store rooted at `dir`, declaring the hash
    /// its objects are named by (spec §5).
    ///
    /// A store records its algorithm once, at creation, in
    /// `store_meta.hash_algo`: every blob is a file named by it, so it
    /// cannot change afterwards. `hash` therefore applies to a store this
    /// call creates, and an existing store whose algorithm differs is
    /// refused with [`PimdirError::HashAlgo`] rather than opened into a
    /// handle that would hash bodies to names it does not use. `None`
    /// adopts what the store records, creating with
    /// [`PimdirHashAlgo::default`].
    ///
    /// A consumer hashes through [`hash`](PimdirReader::hash) or
    /// [`hasher`](PimdirReader::hasher) rather than choosing an algorithm of its
    /// own, which is what keeps two implementations of one store naming
    /// the same body the same way.
    pub fn open_with_hash(
        dir: impl AsRef<Path>,
        hash: Option<PimdirHashAlgo>,
    ) -> Result<Self, PimdirError> {
        let dir = dir.as_ref();
        fs::create_dir_all(dir)?;
        let blobs = dir.join("objects");
        fs::create_dir_all(&blobs)?;

        // NOTE: before the connection, so a store this process may not own is
        // refused before anything is opened, created or migrated in it.
        let lock = PimdirLock::own(dir)?;

        let mut conn = Connection::open(dir.join("pimdir.db"))?;
        // NOTE: `busy_timeout` lets several handles of one store wait out
        // each other's write transaction instead of failing with
        // `SQLITE_BUSY`: §8's single-owner process opening `"left"` and
        // `"right"`, or a sync fanning work across same-source handles.
        // 30s absorbs a burst of large writes contending on the lock.
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 30000;",
        )?;
        init_schema(&mut conn, hash.unwrap_or_default())?;
        let hash = read_hash_algo(&conn, hash)?;

        Ok(Self {
            reader: PimdirReader::over(conn, dir.to_path_buf(), blobs, hash),
            _lock: Some(lock),
            account: None,
        })
    }

    /// Opens an **existing** store rooted at `dir` read-only.
    ///
    /// The database is opened with `SQLITE_OPEN_READ_ONLY`: nothing is
    /// created, so a missing database errors, one no owner has stamped
    /// yet is [`PimdirError::Uncreated`], and any other schema version is
    /// refused with [`PimdirError::Version`]. The returned handle exposes
    /// the full read surface; any write through it fails at the SQLite
    /// layer.
    ///
    /// A reader owns nothing and takes no lock: any number of them may
    /// run against a store an owner holds.
    #[deprecated(
        since = "0.3.0",
        note = "use `PimdirReader::open`, which carries the reads and no write at all"
    )]
    pub fn open_read_only(dir: impl AsRef<Path>) -> Result<Self, PimdirError> {
        let dir = dir.as_ref();
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(dir.join("pimdir.db"), flags)?;
        conn.execute_batch("PRAGMA busy_timeout = 30000;")?;

        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        match version {
            version if version == sql::VERSION => {}
            // NOTE: an unstamped database is one no owner has opened yet, not
            // a version this crate cannot read, and the two want different
            // answers from whoever is holding this handle.
            0 => return Err(PimdirError::Uncreated),
            found => return Err(PimdirError::Version { found }),
        }
        check_version_agreement(&conn, version)?;
        check_rename_cascades(&conn)?;
        let hash = read_hash_algo(&conn, None)?;

        Ok(Self {
            reader: PimdirReader::over(conn, dir.to_path_buf(), dir.join("objects"), hash),
            _lock: None,
            account: None,
        })
    }

    /// Binds this handle to an account, so every collection it creates is
    /// grouped under it (spec §9.2).
    ///
    /// A single-account store never calls this and its collections carry
    /// a `NULL` account, which is what every by-account read matches when
    /// given `None`. A multi-account owner opens one handle per account,
    /// the way it already opens one per source; §8's single-owner rule is
    /// unchanged by how many a process holds.
    ///
    /// The account groups and nothing more: it partitions no identifier,
    /// so two accounts holding one link id still share a `seq`, and one
    /// body reaching both is still stored once. Where an identity or a
    /// body occurs is reported by
    /// [`link_placements`](PimdirReader::link_placements) and
    /// [`object_placements`](PimdirReader::object_placements).
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
    /// `"phone"`, …), so it is only ever named by an operation acting as
    /// one. Everything else, the reads, retention and the queue, stays on
    /// the source-less handle and is still reachable through this one.
    pub fn for_source(self, source: impl Into<String>) -> PimdirSourceStore {
        PimdirSourceStore {
            store: self,
            source: ReplicaSourceId(source.into()),
            residual: HashMap::new(),
        }
    }

    /// Loads a collection's full [`ReplicaHub`]: every source's items and
    /// bindings, not only this handle's source.
    ///
    /// [`load`](ReplicaStorage::load) projects the hub for one source; a
    /// multi-source consumer reads the whole hub to project each side and
    /// to spot items held by a single source.
    pub fn load_hub(&self, collection: &str) -> Result<ReplicaHub, PimdirError> {
        Ok(load_hub(&self.conn, collection)?)
    }

    /// Declares a collection's media type (`kind`), creating the collection if
    /// absent and updating its kind otherwise.
    ///
    /// The kind is an [IANA media
    /// type](https://www.iana.org/assignments/media-types)
    /// (`message/rfc822`, `text/vcard`, `text/calendar`, …), static
    /// consumer configuration rather than something the sync engine
    /// derives, so a consumer sets it out of band from the
    /// [`ReplicaStorage`] seam. That is what makes the store
    /// self-describing (§4.3) and lets one store hold several kinds. The
    /// lazy creation inside [`write`](ReplicaStorage::write) uses
    /// `ON CONFLICT DO NOTHING`, so it never clobbers a kind set here.
    ///
    /// The collection is grouped under this handle's account
    /// ([`for_account`](Self::for_account)); an existing row keeps the
    /// account it had, and only the kind is updated.
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

    /// Restates one item's ordering key (spec §9.3).
    ///
    /// For a re-projection: a store written before its kind had a
    /// sort-key convention, one whose convention changed, or a consumer
    /// deriving the key from the `meta` it wrote itself. Not part of the
    /// ordinary write path, which preserves a key by never naming it.
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
    /// Every foreign key onto `collections(id)` is `ON UPDATE CASCADE`,
    /// so the items, bindings, sources, queue rows and child collections
    /// follow in the same statement (spec §14). This is the only safe way
    /// to change an id: recreating the collection under the new one takes
    /// every item and binding with it through `ON DELETE CASCADE`,
    /// turning a rename into a full re-download and discarding any staged
    /// local change.
    ///
    /// Two things make an id change: a server renaming the collection,
    /// and an owner renaming an account whose id it namespaced its
    /// collection ids with. An account rename is one call per collection;
    /// run them in one transaction and the account moves atomically.
    pub fn rename_collection(&self, collection: &str, new_id: &str) -> Result<(), PimdirError> {
        self.conn.execute(
            sql::RENAME_COLLECTION,
            named_params! { ":collection": collection, ":new_id": new_id },
        )?;
        Ok(())
    }
}

/// The retention surface (spec §11): the trash a store keeps instead of
/// losing items, and the only operations that truly destroy one.
///
/// An item whose last source binding vanished is retained, not deleted:
/// hidden from the sync seam and from the live client reads, but kept
/// whole, body included. It comes back by revival, its link id
/// reappearing from a source or a client `add`, or not at all until a
/// purge reclaims it. Retention is unconditional; when to reclaim is the
/// owner's schedule, which is why every purge takes its boundary from the
/// caller.
impl PimdirStore {
    /// Purges one retained item by its public id, returning whether there was
    /// one to purge.
    ///
    /// The row goes, its bindings cascade, and the body it released is
    /// unlinked by the ordinary sweep once nothing else references it: a
    /// purge collects nothing itself. A live item is never reached, the
    /// statement being guarded on the retention stamp, so an operator
    /// emptying the trash cannot destroy synced data.
    pub fn purge(
        &mut self,
        collection: &ReplicaCollectionId,
        seq: i64,
    ) -> Result<bool, PimdirError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;

        // NOTE: the delete reports the hashes the row pinned, so the
        // release rides the statement that caused it rather than a read
        // visiting the same row first. Nothing to return means there was
        // no retained item under that id, which is how a live one is
        // refused too.
        let pinned: Option<(Option<String>, Option<String>)> = tx
            .prepare(sql::PURGE_ITEM)?
            .query_row(
                named_params! { ":collection": collection.0, ":seq": seq },
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

    /// The scheduled sweep: purges every item retired **strictly before**
    /// `cutoff` (RFC 3339), store-wide, reporting how many it retired.
    ///
    /// The boundary is the caller's, not the store's clock: an owner
    /// computes it from its own retention duration, so the store holds no
    /// policy and the sweep stays deterministic. An item retained exactly
    /// at `cutoff` is kept, and a cutoff of now reproduces the
    /// terminal-delete behaviour of a store that never retained, which is
    /// why there is no on/off switch.
    pub fn purge_retained_before(
        &mut self,
        cutoff: &str,
    ) -> Result<PimdirPurgeReport, PimdirError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;

        // NOTE: one pass. The delete reports what each row it takes was
        // pinning, so the count and the pins to release both come off the
        // statement that did the work.
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

/// Reclamation and repair (spec §5, §7): the two things a store does not
/// do to itself.
///
/// No write collects. An object at refcount zero is unreferenced rather
/// than deleted, and stays until a collector runs, which is what lets a
/// consumer store a body in one batch and attach it in a later one (spec
/// §14). Repair is the other half: a refcount is maintained
/// incrementally, so recomputing it from the pointers that justify it is
/// how a drift is settled rather than reported for ever.
impl PimdirStore {
    /// Reclaims what nothing references: the object rows at refcount zero, the
    /// bodies they held, and any orphan blob a crash left behind.
    ///
    /// Takes the store's staging lock exclusively, so no producer is
    /// between a blob write and the queue row that pins it, and runs on
    /// an owning handle, which already holds the store against other
    /// owners. Those two let the sweep take a body the moment nothing
    /// references it, with no grace window standing in for a lock.
    ///
    /// The rows go inside a transaction and the files after it, in the
    /// order a crash can afford: a body without its row is an orphan the
    /// next collection takes, where a row without its body fails a read.
    pub fn collect_garbage(&mut self) -> Result<PimdirGcReport, PimdirError> {
        let _staging = PimdirLock::collect(&self.dir)?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        let objects = tx.execute(sql::DELETE_GARBAGE_OBJECTS, [])?;
        tx.commit().map_err(busy_or_sql)?;

        // NOTE: one pass over the tree rather than an unlink per
        // collected row plus a pass for the orphans: a body whose row the
        // transaction above dropped is an orphan by now. Asked per file
        // on the primary key rather than read whole into a set, since a
        // store holds hundreds of thousands of hashes and the question is
        // always about one file.
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

    /// Recomputes every object's refcount from the five columns that pin one
    /// (spec §7), returning how many rows disagreed and were corrected.
    ///
    /// The counterpart of the incremental maintenance every write does: a
    /// count that drifted, from a bug here or a foreign writer, is
    /// otherwise reported for ever. A whole-store pass, so it belongs to
    /// a repair verb rather than to a write.
    pub fn recompute_refcounts(&self) -> Result<usize, PimdirError> {
        Ok(self.conn.execute(sql::RECOMPUTE_REFCOUNTS, [])?)
    }

    /// Deletes the bindings whose item is gone, returning how many, and leaves
    /// every other dangling row alone.
    ///
    /// A binding with no item is unreachable: nothing reads it and no
    /// sync projects it. The other dangling rows a check reports are not
    /// like that, an item whose object row is missing being still the
    /// item and a queue row whose body is missing still an intent, so
    /// deleting them would destroy data rather than repair it.
    pub fn clear_dangling_bindings(&self) -> Result<usize, PimdirError> {
        Ok(self.conn.execute(sql::DELETE_DANGLING_BINDINGS, [])?)
    }
}

/// Runs one statement and collects every row through `map`.
///
/// A `Transaction` derefs to a `Connection`, so a read inside a write
/// batch uses this too.
fn rows<T>(
    conn: &Connection,
    sql: &str,
    params: impl Params,
    map: impl FnMut(&Row) -> rusqlite::Result<T>,
) -> rusqlite::Result<Vec<T>> {
    conn.prepare(sql)?.query_map(params, map)?.collect()
}

/// Releases the object references a retained row (or a queue row) held, so the
/// ordinary sweep can reclaim a body nothing points at any more.
fn release_pins(
    conn: &Connection,
    hashes: impl Iterator<Item = String>,
) -> Result<(), PimdirError> {
    // NOTE: one statement rather than one per hash: a purge sweeping
    // fifty thousand retained items releases two pins each, and a point
    // update per pin is a hundred thousand statements to express a set
    // operation.
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

/// The action-queue owner surface (spec §15) and collection generations
/// (spec §12): the single owning process drains producer-requested
/// mutations into the store, and marks a rebuild for readers.
impl PimdirStore {
    /// Cancels one queue row (spec §15.5) as the store's owner, holding
    /// that role only for the length of the call.
    ///
    /// Cancelling is an owner write, and it is the only retraction a
    /// queued create has: the kinds that address an existing item are
    /// retracted by their inverse instead, `set-flags` being absolute
    /// rather than a delta. A consumer that is otherwise a reader and a
    /// producer needs the role for this one statement, so it takes it
    /// here rather than by holding a handle that could also drain the
    /// queue or sweep the objects.
    ///
    /// The store must exist: this never creates one, so a mistyped path
    /// is [`PimdirError::Uncreated`] rather than an empty store. A store
    /// another process owns is [`PimdirError::Owned`] at once, never a
    /// wait, and the caller reports it as a sync being in flight: the
    /// action is still queued, and may have been applied in the meantime.
    pub fn cancel_action(dir: impl AsRef<Path>, id: i64) -> Result<bool, PimdirError> {
        let dir = dir.as_ref();
        if !dir.join("pimdir.db").is_file() {
            return Err(PimdirError::Uncreated);
        }

        Self::open(dir)?.drop_action(id)
    }

    /// Removes one queue row by request rather than by application, pending or
    /// parked, returning whether there was a row to remove (spec §15.5).
    ///
    /// One verb for the two ways a row leaves the queue unapplied: a
    /// producer cancelling a queued action, and an owner acknowledging an
    /// intent it performed out of band, which the drain could only skip.
    /// The row's body pin is released in the same transaction, so a blob
    /// nothing else references falls to the ordinary sweep.
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
        tx.commit().map_err(busy_or_sql)?;
        Ok(true)
    }

    /// Records a failed apply an owner performed itself (spec §15.2).
    ///
    /// `None` is the transient case: the attempt counter advances and the
    /// row stays pending for the next drain. `Some(error)` is the
    /// permanent one: the row parks with the failure, visible to
    /// operators instead of blocking its collection. An unknown id is a
    /// no-op, since the row may have been applied or cancelled meanwhile.
    pub fn fail_action(&self, id: i64, error: Option<&str>) -> Result<(), PimdirError> {
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

/// The sync seam and what only a side can mean: the source-bound writes,
/// and the drain that stages a producer's queued mutation for that side.
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
    /// The owner drives io-replica's rekey coroutine and routes its
    /// rebuild writes here rather than to
    /// [`write`](ReplicaStorage::write), so "the ids you cached are void"
    /// commits atomically with the rebuild that voided them. Ordinary
    /// syncs, full resyncs and content changes never bump.
    pub fn write_rekeyed(
        &mut self,
        collection: &str,
        ops: Vec<ReplicaWriteOp>,
    ) -> Result<i64, PimdirError> {
        // NOTE: as in `write`, the bodies land before the transaction opens.
        stage_blobs(&self.store.reader.blobs, &ops)?;

        let tx = self
            .store
            .reader
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        apply_ops(
            &tx,
            &self.store.reader.blobs,
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
        tx.commit().map_err(busy_or_sql)?;
        Ok(generation)
    }

    /// Drains a collection's pending actions in append order (spec §15.2).
    ///
    /// Each action is applied as the store mutation it names, resolving
    /// its public `seq` to the internal link id and folding the
    /// corresponding io-replica mutation through the store's own write
    /// machinery, and its row is deleted in the same transaction, so
    /// application is exactly-once and never partially visible. An action
    /// the owner judges permanently unappliable is parked with its error
    /// and skipped; a transient failure increments the row's `attempts`
    /// and stops the pass, preserving apply order for the retry.
    ///
    /// An action whose kind this store defines no semantics for is
    /// skipped: left pending, never parked, never blocking the actions
    /// behind it. That is what lets one queue carry store mutations any
    /// owner applies beside capability-bound intents only a specific
    /// owner can perform; that owner reads the row through
    /// [`pending_actions`](PimdirReader::pending_actions), performs it,
    /// and acknowledges it with
    /// [`drop_action`](PimdirStore::drop_action).
    pub fn drain_collection(&mut self, collection: &str) -> Result<PimdirDrainReport, PimdirError> {
        let pending: Vec<QueueRow> = rows(
            &self.store.reader.conn,
            sql::LOAD_PENDING_ACTIONS,
            named_params! { ":collection": collection },
            |r| {
                Ok(QueueRow {
                    id: r.get(0)?,
                    action: r.get(3)?,
                    payload: r.get(4)?,
                    object_hash: r.get(5)?,
                })
            },
        )?;

        let mut report = PimdirDrainReport::default();
        for row in pending {
            let action = match codec::action_from_payload(&row.action, &row.payload) {
                Ok(action) => action,
                Err(err) => {
                    self.fail_action(row.id, Some(&err.to_string()))?;
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
                Ok(Some(PimdirRefusal::Skip)) => report.skipped += 1,
                Ok(Some(PimdirRefusal::Park(reason))) => {
                    self.fail_action(row.id, Some(&reason))?;
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
    /// releasing the row's object pin as the applied item takes its own.
    /// Returns `None` when applied, and the [`PimdirRefusal`] otherwise,
    /// rolling the transaction back so the row is as it was.
    fn apply_queued(
        &mut self,
        collection: &str,
        row: &QueueRow,
        action: &PimdirAction,
    ) -> Result<Option<PimdirRefusal>, PimdirError> {
        let tx = self
            .store
            .reader
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;

        // NOTE: claim the row before doing its work. The pending rows
        // were read outside any transaction, so another owner may have
        // applied this one already and `add` or `copy` would land twice.
        // A claim that deletes nothing means exactly that.
        let claimed = tx
            .prepare(sql::CLAIM_ACTION)?
            .query_row(named_params! { ":id": row.id }, |r| r.get::<_, i64>(0))
            .optional()?;
        if claimed.is_none() {
            return Ok(None);
        }

        let ops = match stage_action(&tx, &self.source, collection, row.id, action)? {
            Ok(ops) => ops,
            // NOTE: dropping the transaction rolls the attempt back, so a
            // skipped row is left exactly as it was found: still pending,
            // its attempts untouched, for the owner that can apply it.
            Err(refusal) => return Ok(Some(refusal)),
        };
        apply_ops(
            &tx,
            &self.store.reader.blobs,
            &self.source,
            self.store.account.as_deref(),
            &mut self.residual,
            ops,
        )?;
        // NOTE: the pin hand-over: the queue row's reference, taken at
        // enqueue, is released as the row goes, while the applied item's
        // own was just taken by `apply_ops`, both in this transaction, so
        // a queued body is never sweepable in between.
        if let Some(hash) = &row.object_hash {
            tx.execute(
                sql::ADJUST_REFCOUNT,
                named_params! { ":delta": -1, ":hash": hash },
            )?;
        }
        tx.commit().map_err(busy_or_sql)?;
        Ok(None)
    }
}

impl ReplicaStorage for PimdirSourceStore {
    type Error = PimdirError;

    fn load(
        &self,
        collection: &ReplicaCollectionId,
        scope: &ReplicaLoadScope,
    ) -> Result<ReplicaLoaded, Self::Error> {
        // NOTE: the scope narrows the hub read, and the projection only
        // produces placements for what was read. A handle scope cannot
        // narrow the query, the hub being keyed by link id, so a handle
        // is resolved through its binding first and one no binding holds
        // contributes nothing.
        let hub = match scope {
            ReplicaLoadScope::All => load_hub(&self.store.reader.conn, &collection.0)?,
            ReplicaLoadScope::Links(links) => {
                let links: Vec<String> = links.iter().map(|l| l.0.clone()).collect();
                load_hub_by_link(&self.store.reader.conn, &collection.0, &links)?
            }
            ReplicaLoadScope::Handles(handles) => {
                let mut links = Vec::new();
                for handle in handles {
                    links.extend(link_for_handle(
                        &self.store.reader.conn,
                        &collection.0,
                        &self.source,
                        handle,
                    )?);
                }
                load_hub_by_link(&self.store.reader.conn, &collection.0, &links)?
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
            .reader
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

        let found = rows(
            &self.store.reader.conn,
            sql::LOOKUP_OBJECTS,
            named_params! { ":links": json, ":account": self.store.account.as_deref() },
            |r| {
                Ok((
                    ReplicaLinkId(r.get::<_, String>(0)?),
                    ReplicaHash(r.get::<_, String>(1)?),
                ))
            },
        )?;
        let mut map: BTreeMap<ReplicaLinkId, ReplicaHash> = found.into_iter().collect();

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
        // NOTE: bodies first, outside the transaction, so the writer lock
        // is never held across a file write and two `fsync`s (spec §14).
        stage_blobs(&self.store.reader.blobs, &ops)?;

        // NOTE: BEGIN IMMEDIATE takes the single writer lock up front
        // (§8), so a writer that cannot get it within `busy_timeout`
        // fails fast with `Busy` rather than deep inside the batch on a
        // deferred lock upgrade.
        let tx = self
            .store
            .reader
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        apply_ops(
            &tx,
            &self.store.reader.blobs,
            &self.source,
            self.store.account.as_deref(),
            &mut self.residual,
            ops,
        )?;
        tx.commit().map_err(busy_or_sql)?;
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

/// One raw pending queue row, as the drain loads it: the payload
/// undecoded, so a malformed one parks instead of failing the pass.
struct QueueRow {
    id: i64,
    action: String,
    payload: String,
    object_hash: Option<String>,
}

/// Loads a collection's pending actions in append order, decoding each payload
/// strictly. Shared by [`PimdirStore::pending_actions`] and
/// [`PimdirProducer::pending_actions`].
fn load_pending_actions(
    conn: &Connection,
    collection: &str,
) -> Result<Vec<PimdirPendingAction>, PimdirError> {
    let pending = rows(
        conn,
        sql::LOAD_PENDING_ACTIONS,
        named_params! { ":collection": collection },
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(6)?,
            ))
        },
    )?;

    let mut actions = Vec::new();
    for (id, created_at, producer, kind, payload, attempts) in pending {
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

/// Why a queued action was not staged.
///
/// The distinction is the whole difference between a row that is broken
/// and one that is simply not this owner's to apply, and it is the spec's
/// (§15.3): an action the owner cannot apply **at all** parks, one it
/// cannot apply **here** is left pending for whoever can.
enum PimdirRefusal {
    /// It will never stage: a payload that does not decode, an item that
    /// is gone, an identity already taken. The row parks with this
    /// reason, and no later drain retries it.
    Park(String),
    /// It cannot stage against this source, which holds no binding for
    /// the item it names. The row stays pending, unmarked, so the source
    /// that does hold one still applies it.
    Skip,
}

/// Stages the io-replica write ops one queued action folds into the store
/// (spec §15.3), inside the drain transaction. The inner `Err` is why it
/// was refused; an empty op list is a no-op success, a `remove` of an
/// already-absent item.
///
/// Existing items are addressed by `seq`, resolved to their link id and
/// then to this source's projected placement, and the matching
/// [`ReplicaMutation`] is pumped through the real [`ReplicaMutate`]
/// coroutine, so the staging semantics stay the engine's. An `add` is
/// staged directly as the `Created` placement the engine's `Add` stages,
/// minus the body bytes: the producer wrote the blob at enqueue.
fn stage_action(
    tx: &Connection,
    source: &ReplicaSourceId,
    collection: &str,
    row_id: i64,
    action: &PimdirAction,
) -> Result<Result<Vec<ReplicaWriteOp>, PimdirRefusal>, PimdirError> {
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
            return Ok(Err(PimdirRefusal::Park(
                "add carries neither link_id nor object".to_string(),
            )));
        };
        // NOTE: the same collision rule as the engine's Add mutation: a
        // live item blocks the create, a tombstone does not, the delete
        // being in flight. Asked of the one row that could collide, since
        // this runs once per drained action and loading the collection
        // would make a drain of N actions cost N passes over the mailbox.
        let live = tx
            .query_row(
                sql::LIVE_ITEM_FOR_LINK,
                named_params! { ":collection": collection, ":link_id": link.0 },
                |r| r.get::<_, i64>(0),
            )
            .optional()?;
        if live.is_some() {
            return Ok(Err(PimdirRefusal::Park(format!(
                "link id already present: {}",
                link.0
            ))));
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
            // sort key; the sync pushing this create resolves one.
            sort_key: ReplicaSortKey::default(),
            flags: flags.clone(),
            status: ReplicaStatus::Created,
            conflict_revision: None,
            conflict_object: None,
            base: None,
            origin: None,
        };
        return Ok(Ok(vec![ReplicaWriteOp::UpsertPlacement(create)]));
    }

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
        // NOTE: a remove of an already-absent item is success, not an
        // error (spec §15.3); anything else addressing a gone item parks.
        return if removes {
            Ok(Ok(Vec::new()))
        } else {
            Ok(Err(PimdirRefusal::Park(format!("unknown seq: {seq}"))))
        };
    };

    // NOTE: the binding's own primary key answers this, so it is a seek.
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
        None => return Ok(Err(PimdirRefusal::Skip)),
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
            // NOTE: the size only rides the StoreObject op, stripped
            // below; the object row was indexed with its real size at
            // enqueue.
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

    // NOTE: the mutation reads one placement, so the hub is read for the
    // one identity it names rather than for the collection.
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
            // NOTE: the body already sits in the blob store and its
            // object row was upserted and pinned at enqueue, so
            // re-storing would clobber the recorded size.
            let ops = ops
                .into_iter()
                .filter(|op| !matches!(op, ReplicaWriteOp::StoreObject { .. }))
                .collect();
            Ok(Ok(ops))
        }
        ReplicaCoroutineState::Complete(Err(err)) => Ok(Err(PimdirRefusal::Park(err.to_string()))),
        state => Ok(Err(PimdirRefusal::Park(format!(
            "unexpected mutate state: {state:?}"
        )))),
    }
}

/// A pimdir store opened as a producer (spec §8): a process that is not
/// the owner but legitimately originates mutations (a submission daemon,
/// a server frontend). Its only write is the single enqueue transaction
/// of spec §15.1: `ensure_collection`, at most one object upsert pinning
/// a body it already wrote durably through [`PimdirBlobs::writer`], and
/// one queue insert. It never touches items, bindings or sources, and
/// never creates the schema: it requires a store the owner has already
/// opened at the current version.
///
/// This coexists with the store's single-writer serialisation: the guard
/// is the per-transaction `BEGIN IMMEDIATE` plus the busy timeout, and
/// the spec sanctions the producer's short append transaction beside the
/// owner's batches, the two serialising on the write lock.
pub struct PimdirProducer {
    conn: Connection,
    /// The store's shared staging lock (spec §8), held for this handle's
    /// lifetime so a body written before an enqueue and the row pinning
    /// it are one window a collector cannot run inside.
    _lock: PimdirLock,
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
    /// The database must exist at the current schema version: a producer
    /// never creates a store, so a missing database errors and a version
    /// mismatch is [`PimdirError::Version`].
    ///
    /// A producer is not an owner: it takes the store's shared lock (spec
    /// §8), so several run at once and none keeps the owner out. What the
    /// lock buys is the window a collector must not run inside, between
    /// the blob write and the queue row that pins it, so a producer
    /// handle is opened for the staging it is about to do and dropped
    /// when that is done.
    pub fn open(dir: impl AsRef<Path>, producer: impl Into<String>) -> Result<Self, PimdirError> {
        let dir = dir.as_ref();
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(dir.join("pimdir.db"), flags)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 30000;",
        )?;

        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        match version {
            version if version == sql::VERSION => {}
            // NOTE: an unstamped database is one no owner has opened yet, not
            // a version this crate cannot read, and the two want different
            // answers from whoever is holding this handle.
            0 => return Err(PimdirError::Uncreated),
            found => return Err(PimdirError::Version { found }),
        }
        check_version_agreement(&conn, version)?;
        check_rename_cascades(&conn)?;
        let hash = read_hash_algo(&conn, None)?;

        Ok(Self {
            conn,
            _lock: PimdirLock::stage(dir)?,
            producer: producer.into(),
            hash,
            account: None,
        })
    }

    /// The hash this store names its objects by (spec §5).
    pub fn hash_algo(&self) -> PimdirHashAlgo {
        self.hash
    }

    /// The content hash of a whole body, under this store's algorithm:
    /// what a producer names the blob it writes before enqueueing the
    /// action referencing it (spec §15.1).
    pub fn hash(&self, bytes: &[u8]) -> ReplicaHash {
        self.hash.hash(bytes)
    }

    /// An incremental hasher for a body streamed into the blob store.
    pub fn hasher(&self) -> PimdirHasher {
        self.hash.hasher()
    }

    /// Binds this producer to an account, so a collection its enqueue
    /// creates is grouped under it (spec §9.2). Mirrors
    /// [`PimdirStore::for_account`].
    pub fn for_account(mut self, account: impl Into<String>) -> Self {
        self.account = Some(account.into());
        self
    }

    /// Appends one action to a collection's queue (spec §15.1), returning the
    /// row's append id.
    ///
    /// Runs exactly the producer transaction, `BEGIN IMMEDIATE` and
    /// short: `ensure_collection`, at most one object upsert when the
    /// payload references a body, and one queue insert pinning that
    /// body's hash against garbage collection. When the action carries an
    /// object the caller has already written its blob durably through
    /// [`PimdirBlobs::writer`] and passes the byte size the commit
    /// returned; `None` reuses an object the store already indexes.
    /// `created_at` is the caller's RFC 3339 timestamp. When the owner
    /// applies the action is the owner's business.
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
        // NOTE: the pin (+1): the queue row now references the body, so
        // garbage collection never sweeps it between enqueue and apply,
        // and the drain releases it as the row is deleted.
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

    /// The collection's pending (non-parked) actions in append order, the
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
/// A body can be read through it while the [`PimdirStore`] is mutably
/// borrowed to service a sync, a remote reading a stored body back to
/// re-upload it. Cheap to clone: it wraps only the `objects/` path.
#[derive(Clone, Debug)]
pub struct PimdirBlobs {
    root: PathBuf,
    hash: PimdirHashAlgo,
}

impl PimdirBlobs {
    /// Opens the blob handle for the store rooted at `dir`, naming bodies with
    /// `hash`.
    ///
    /// The algorithm is the store's, not a choice made here: it is what
    /// the files are named by. [`PimdirReader::blobs`] hands one out
    /// already bound to the store it came from.
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

    /// Where a body under `hash` lives: `objects/<name[0:2]>/<name[2:4]>/<name>`
    /// (spec §5).
    ///
    /// Public because the format invites a consumer to stream a body
    /// straight to this path and index it with a byteless `StoreObject`
    /// afterwards (spec §14), and deriving the sharding itself would be a
    /// second implementation of a rule whose point is that one store's
    /// writers agree on it.
    pub fn path(&self, hash: &ReplicaHash) -> PathBuf {
        blob_path(&self.root, &hash.0)
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

    /// Opens a stored object as a readable stream, or `None` when absent:
    /// the append side of bounded-memory transfer, so a body is uploaded
    /// without being read whole into memory. The file's metadata gives
    /// the octet length IMAP `APPEND` needs up front.
    pub fn reader(&self, hash: &ReplicaHash) -> io::Result<Option<fs::File>> {
        match fs::File::open(blob_path(&self.root, &hash.0)) {
            Ok(file) => Ok(Some(file)),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// Opens a streaming writer for a new object: bytes go to a temporary
    /// file and reach their content-addressed path only on
    /// [`commit`](PimdirBlobWriter::commit), once the hash is known. The
    /// caller hashes the bytes as it writes them.
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

    /// Every body the blob tree holds, walking the two-level sharding.
    ///
    /// The files, not the index: what a collector and a consistency check
    /// compare the object rows against, the difference either way being a
    /// defect. A half-written body is skipped, a temp file belonging to a
    /// writer that has not committed.
    pub fn files(&self) -> io::Result<Vec<PimdirBlobFile>> {
        let mut files = Vec::new();
        if self.root.is_dir() {
            walk_blobs(&self.root, &mut files)?;
        }
        Ok(files)
    }
}

/// One body as it sits in the blob tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirBlobFile {
    /// The hash its filename claims, unverified: checking it against the
    /// bytes is what `pimdir check` is for.
    pub hash: String,
    /// Where it sits.
    pub path: PathBuf,
    /// Its size on disk.
    pub size: u64,
}

/// Recurses one directory of the blob tree.
fn walk_blobs(dir: &Path, files: &mut Vec<PimdirBlobFile>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }

        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            walk_blobs(&entry.path(), files)?;
        } else if metadata.is_file() {
            files.push(PimdirBlobFile {
                hash: name,
                path: entry.path(),
                size: metadata.len(),
            });
        }
    }

    Ok(())
}

/// A unique-per-write temp-file discriminator, so concurrent writers of
/// one store do not collide on the staging file.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// A streaming writer for one new blob (see [`PimdirBlobs::writer`]).
///
/// A [`Write`] sink over a temporary file; [`commit`](Self::commit)
/// fsyncs and renames it into the content-addressed path once the caller
/// knows the hash. Dropped without a commit, it removes the temp.
pub struct PimdirBlobWriter {
    root: PathBuf,
    tmp: PathBuf,
    file: Option<fs::File>,
    written: u64,
}

impl PimdirBlobWriter {
    /// Finalises the object under `hash`: fsync, then atomically rename
    /// the temp file into its sharded content-addressed path. A body
    /// already present keeps the stored copy and drops the temp. Returns
    /// the object's byte size.
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
        if self.file.is_some() {
            let _ = fs::remove_file(&self.tmp);
        }
    }
}

/// Applies a write batch's ops inside the caller's transaction: blob and
/// object writes, checkpoint upserts, and placement ops folded per
/// collection through the hub. The fold absorbs, then persists only what
/// changed, diffing the loaded hub against the absorbed one and adjusting
/// object refcounts by the per-hash change alone, never a
/// whole-collection rewrite or a global recompute.
///
/// Shared by the seam's [`write`](ReplicaStorage::write), the rekey write
/// ([`PimdirStore::write_rekeyed`]) and the queue drain
/// ([`PimdirStore::drain_collection`]), each wrapping the same folding in
/// its own transaction shape.
fn apply_ops(
    tx: &Connection,
    blobs: &Path,
    source: &ReplicaSourceId,
    account: Option<&str>,
    residual: &mut HashMap<(ReplicaCollectionId, ReplicaHandle), ReplicaPlacement>,
    ops: Vec<ReplicaWriteOp>,
) -> Result<(), PimdirError> {
    let mut hub_ops: BTreeMap<String, Vec<ReplicaWriteOp>> = BTreeMap::new();
    // NOTE: the handles this batch replaces rather than removes.
    let mut superseded: BTreeMap<String, BTreeSet<ReplicaHandle>> = BTreeMap::new();

    for op in ops {
        match op {
            ReplicaWriteOp::StoreObject { object, body } => {
                // NOTE: a byteless op indexes an object the consumer
                // already streamed into the blob store. Inline bytes are
                // normally staged by `stage_blobs` before this
                // transaction opened, and re-offered here as the floor
                // for a caller that could not stage ahead; the write is
                // idempotent, so that costs one `exists` check.
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
            ReplicaWriteOp::UpsertPlacement(mut placement) => {
                // NOTE: an upsert carrying no link id against a handle a
                // binding already holds restates that item rather than
                // announcing a new one: a sync reprobes a resurrected
                // handle with no identity (io-replica, `pull_add`).
                // Keying it back onto its binding folds it into the row
                // the handle is bound to, instead of filing a second row
                // beside it that `load` would then answer with (§10).
                if placement.link_id.is_none() {
                    placement.link_id =
                        link_for_handle(tx, &placement.collection.0, source, &placement.handle)?
                            .map(ReplicaLinkId);
                }
                if placement.link_id.is_some() {
                    drop_residual(residual, &placement.collection, &placement.handle);
                    hub_ops
                        .entry(placement.collection.0.clone())
                        .or_default()
                        .push(ReplicaWriteOp::UpsertPlacement(placement));
                } else {
                    // NOTE: not yet linked, so it stages in the residual
                    // until a Meta upgrade resolves its link id.
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
                // NOTE: a superseded handle is one the batch is
                // replacing, so its binding may legitimately be repointed
                // at whatever the same batch upserts. Recorded here
                // because the hub diff cannot tell that from a source
                // reporting one identity under a second handle, and
                // refuses the second (§10, §12).
                if reason == ReplicaDropReason::Superseded {
                    superseded
                        .entry(collection.0.clone())
                        .or_default()
                        .insert(handle.clone());
                }
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
        refuse_colliding_upserts(&collection, source, &ops)?;
        let links = batch_links(tx, &collection, source, &ops)?;
        let old_hub = load_hub_by_link(tx, &collection, &links)?;
        let mut new_hub = old_hub.clone();
        new_hub.absorb(source, &ops);
        let superseded = superseded.remove(&collection).unwrap_or_default();
        save_hub_diff(
            tx,
            &collection,
            source,
            account,
            &old_hub,
            &new_hub,
            &superseded,
        )?;
        adjust_refcounts(tx, &object_refs(&old_hub), &object_refs(&new_hub))?;
    }

    Ok(())
}

/// Refuses a batch carrying two placements of one collection under one
/// link id and two handles, before any of it is folded.
///
/// The hub is keyed by link id, so absorbing both would keep whichever
/// the batch names last and drop the other with no statement failing.
/// The engine mints a key for the second copy it reads from a source
/// (spec §9), but a handle-space rebuild re-resolves every identity from
/// the new spine and mints none, so a collection that genuinely holds a
/// duplicate hands this store two placements resolving to one key. It is
/// the collision [`save_bindings_diff`] refuses against a stored binding,
/// one write earlier and against the batch itself.
fn refuse_colliding_upserts(
    collection: &str,
    source: &ReplicaSourceId,
    ops: &[ReplicaWriteOp],
) -> Result<(), PimdirError> {
    let mut claimed: BTreeMap<&ReplicaLinkId, &ReplicaHandle> = BTreeMap::new();

    for op in ops {
        let ReplicaWriteOp::UpsertPlacement(placement) = op else {
            continue;
        };
        let Some(link) = placement.link_id.as_ref() else {
            continue;
        };
        match claimed.insert(link, &placement.handle) {
            Some(bound) if *bound != placement.handle => {
                return Err(PimdirError::Rebind {
                    collection: collection.into(),
                    link_id: link.0.clone(),
                    source: source.0.clone(),
                    bound: bound.0.clone(),
                    incoming: placement.handle.0.clone(),
                });
            }
            _ => {}
        }
    }

    Ok(())
}

/// Creates the schema in a fresh database (spec §6), advancing
/// `user_version` and seeding `store_meta.version` in agreement (spec
/// §4.2) in one transaction. A store stamped higher than
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
    // NOTE: the canonical script is pure DDL, so `store_meta`'s one row
    // is seeded here. The timestamp is SQLite's own, in the RFC 3339 form
    // the column is declared to hold, which keeps the crate free of a
    // clock.
    tx.execute(
        "INSERT OR IGNORE INTO store_meta(id, version, hash_algo, created_at) \
         VALUES(1, ?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![sql::VERSION, hash.as_str()],
    )?;
    tx.pragma_update(None, "user_version", sql::VERSION)?;
    tx.commit().map_err(busy_or_sql)?;

    Ok(())
}

/// Refuses a store whose foreign keys predate the `ON UPDATE CASCADE`
/// every key onto a renamed row now carries (spec §14).
///
/// The half of the draft allowance (spec §6) that reconciliation cannot
/// reach: a column can be added in place, a foreign-key action cannot.
/// §6's other branch is to refuse the store and have the operator
/// recreate it, which costs a resync of a derived cache.
///
/// Without the cascade SQLite refuses a rename one dependent row down, so
/// such a store can never follow a server-side collection rename;
/// catching it on open says so once rather than when a rename fails.
fn check_rename_cascades(conn: &Connection) -> Result<(), PimdirError> {
    /// The tables whose foreign key onto a renamable parent must cascade,
    /// with that parent. `bindings` hangs off `items(collection,
    /// link_id)`, which the first cascade updates.
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

/// Adds columns folded into version 1 after a store was already created
/// at version 1 (spec §6, the `draft` allowance).
///
/// While the spec is a draft, version 1 is not frozen: a schema change
/// may be folded into `0001_init.sql` rather than added as version 2. The
/// cost is that such a store is not detectably out of date, its
/// `user_version` already matching, so the missing column would surface
/// later as a query error. §6 requires an implementation to reconcile the
/// shape on open or refuse the store; this reconciles.
///
/// `ALTER TABLE … ADD COLUMN` is cheap, and guarding on `PRAGMA
/// table_info` makes it a no-op for a current store. Only nullable
/// columns or ones carrying a default can be folded in this way. A draft
/// may also fold a column back *out*, which the same guard reverses into
/// a `DROP COLUMN`, so a store carrying one the format has retired stops
/// carrying it.
///
/// This disappears when the spec leaves `draft`: from the first frozen
/// version onwards, a shape change is a numbered migration.
fn reconcile_draft_shape(conn: &mut Connection) -> Result<(), PimdirError> {
    /// Columns folded into version 1 after it was first published, as
    /// `(table, column, declaration)`. Each must be nullable or carry a
    /// default, or it could not be added to a populated table.
    const FOLDED_IN: [(&str, &str, &str); 9] = [
        ("bindings", "conflicted", "INTEGER NOT NULL DEFAULT 0"),
        ("bindings", "conflict_revision", "TEXT"),
        (
            "bindings",
            "conflict_object",
            "TEXT REFERENCES objects(hash)",
        ),
        ("bindings", "shared_object", "TEXT"),
        ("items", "retained_at", "TEXT"),
        ("items", "retained_by", "TEXT"),
        ("collections", "account", "TEXT"),
        ("items", "sort_key", "TEXT NOT NULL DEFAULT ''"),
        ("bindings", "base_present", "INTEGER NOT NULL DEFAULT 0"),
    ];

    /// Columns a later draft folded back out, as `(table, column)`.
    ///
    /// `bindings.ambiguous_handles` held the handles a source held one
    /// identity under; the second copy is an item of its own now (spec
    /// §9), so the column has nothing to hold and the store records no
    /// trace of an incoming handle. A store written with it keeps rows
    /// stating a rule the crate no longer has.
    ///
    /// Dropped in place rather than through the table rebuild §6
    /// prescribes for a constraint: no index, key, foreign key or check
    /// names this column, so `ALTER TABLE` expresses the change whole,
    /// and rebuilding would mean a second copy of the canonical
    /// `bindings` DDL for the reconciliation to drift from.
    const FOLDED_OUT: [(&str, &str); 1] = [("bindings", "ambiguous_handles")];

    let mut missing = Vec::new();
    for (table, column, decl) in FOLDED_IN {
        if !has_column(conn, table, column)? {
            missing.push((table, column, decl));
        }
    }

    let mut stale = Vec::new();
    for (table, column) in FOLDED_OUT {
        if has_column(conn, table, column)? {
            stale.push((table, column));
        }
    }

    let mut reshaped = Vec::new();
    for (index, columns) in sql::RESHAPED_INDEXES {
        if index_columns(conn, index)?.is_some_and(|held| held != *columns) {
            reshaped.push(*index);
        }
    }

    let backfill_shared = missing
        .iter()
        .any(|(table, column, _)| (*table, *column) == ("bindings", "shared_object"));

    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(busy_or_sql)?;
    for (table, column, decl) in missing {
        tx.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"))?;
    }
    // NOTE: the one folded-in column whose empty value is not what the
    // rows already say. Left NULL it reads as "never folded", the sync
    // base stands in for it, and a binding with a pending push sits
    // behind the shared body by definition, so the first absorb after
    // the upgrade files the source's own next edit as a divergence.
    if backfill_shared {
        tx.execute_batch(sql::BACKFILL_SHARED_OBJECT)?;
    }
    for (table, column) in stale {
        tx.execute_batch(&format!("ALTER TABLE {table} DROP COLUMN {column}"))?;
    }
    // NOTE: before the batch below, which cannot replace it: an index
    // whose columns moved keeps its name, so `CREATE INDEX IF NOT EXISTS`
    // sees one already there and leaves the old plan in place.
    for index in reshaped {
        tx.execute_batch(&format!("DROP INDEX IF EXISTS {index}"))?;
    }
    // NOTE: unconditionally, unlike the columns: most of these index
    // columns that were always there, and what changed is that a
    // statement now needs them. A store keeping the old plans would scan
    // where the schema says it seeks.
    tx.execute_batch(sql::ENSURE_INDEXES)?;
    tx.commit().map_err(busy_or_sql)?;
    Ok(())
}

/// The algorithm the store records, checked against the one the caller
/// declared.
///
/// A store names every blob by its hash, so a handle computing a
/// different one writes bodies no reader finds and dedups against
/// nothing. The failure is silent by nature, which is why it is caught on
/// open rather than left to surface as a cache that never hits.
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

/// The two schema stamps a store carries, which spec §4.2 requires to
/// agree: `PRAGMA user_version` and `store_meta.version`. A store where
/// they differ is corrupt, so it is refused rather than read at the
/// version one of them names.
///
/// A store whose `store_meta` row is absent is left alone: the row is
/// seeded by whoever created the schema, and refusing here would turn a
/// missing stamp into an unopenable store the crate could repair.
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

/// The columns `index` holds, in order, or `None` when the store has no
/// such index.
///
/// `PRAGMA index_info` reports the columns and not the partial predicate,
/// which is all a reshape check needs: the column order decides whether a
/// read seeks or sorts.
fn index_columns(conn: &Connection, index: &str) -> rusqlite::Result<Option<Vec<String>>> {
    let columns = rows(conn, &format!("PRAGMA index_info({index})"), [], |row| {
        row.get::<_, String>(2)
    })?;

    Ok((!columns.is_empty()).then_some(columns))
}

/// Removes any residual placement matching `(collection, handle)`.
fn drop_residual(
    residual: &mut HashMap<(ReplicaCollectionId, ReplicaHandle), ReplicaPlacement>,
    collection: &ReplicaCollectionId,
    handle: &ReplicaHandle,
) {
    residual.remove(&(collection.clone(), handle.clone()));
}

/// The link id one source's handle is bound to, if any: the hub is keyed
/// by link id, so a write or a read naming a handle resolves it first.
fn link_for_handle(
    conn: &Connection,
    collection: &str,
    source: &ReplicaSourceId,
    handle: &ReplicaHandle,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        sql::LINK_FOR_HANDLE,
        named_params! {
            ":collection": collection,
            ":source": source.0,
            ":handle": handle.0,
        },
        |r| r.get::<_, String>(0),
    )
    .optional()
}

/// Loads a collection's [`ReplicaHub`] (items + per-source bindings + policy).
fn load_hub(conn: &Connection, collection: &str) -> rusqlite::Result<ReplicaHub> {
    read_hub(conn, collection, None)
}

/// The link ids one write batch touches: the ones its upserts carry, plus
/// the ones its drops resolve to, a drop naming a handle where the shared
/// item is keyed by link id.
///
/// A handle no binding holds resolves to nothing and is left out: there
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
                links.extend(link_for_handle(conn, collection, source, handle)?);
            }
            _ => {}
        }
    }

    Ok(links.into_iter().collect())
}

/// The hub narrowed to `links`, which is what a write folds its batch into.
///
/// The batch only produces writes for the items it names, so the rest of
/// the collection would be read, cloned and diffed to conclude that
/// nothing changed: one flag on one message would cost the size of the
/// mailbox. Both sides of the diff are narrowed the same way, so every
/// comparison and every counted object reference sees what it would have
/// seen in full.
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

    // NOTE: the scoped statements name a `:links` the unscoped ones do
    // not, and a parameter a statement never declared is an error, so the
    // scope is bound only when there is one.
    let scope = links.map(|links| serde_json::to_string(links).unwrap_or_else(|_| "[]".into()));
    let (items_sql, bindings_sql) = match scope {
        Some(_) => (sql::LOAD_ITEMS_BY_LINK, sql::LOAD_BINDINGS_BY_LINK),
        None => (sql::LOAD_ITEMS, sql::LOAD_BINDINGS),
    };
    let mut params: Vec<(&str, &dyn ToSql)> = vec![(":collection", &collection)];
    if let Some(scope) = &scope {
        params.push((":links", scope));
    }

    for (link, item) in rows(conn, items_sql, params.as_slice(), item_from_row)? {
        hub.items.insert(link, item);
    }
    for (link, source, binding) in rows(conn, bindings_sql, params.as_slice(), binding_from_row)? {
        if let Some(item) = hub.items.get_mut(&link) {
            item.sources.insert(source, binding);
        }
    }

    Ok(hub)
}

/// Persists the change from `old` to `new` for a collection's hub by
/// diffing the two in memory and issuing only the item and binding
/// writes that differ, never a whole-collection delete-and-reinsert.
///
/// Paired with a batch-scoped read ([`load_hub_by_link`]) that makes both
/// halves of a write proportional to the batch rather than to the
/// collection. An item no source holds any more is retained rather than
/// deleted, `source` naming the side whose removal retired it.
///
/// `superseded` carries the handles this batch is replacing, the one
/// thing the two hubs cannot say: a rebuilt spine and a duplicated
/// identity produce the same diff, and only the drop's reason separates
/// them.
fn save_hub_diff(
    conn: &Connection,
    collection: &str,
    source: &ReplicaSourceId,
    account: Option<&str>,
    old: &ReplicaHub,
    new: &ReplicaHub,
    superseded: &BTreeSet<ReplicaHandle>,
) -> Result<(), PimdirError> {
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

    // NOTE: an item no source holds any more is retained rather than
    // deleted, a store losing one only to a purge. The bindings go with
    // the sources that held them; the row stays, hidden from `LOAD_ITEMS`
    // so no later sync re-derives against it.
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
        // NOTE: the caller's refcount diff is about to release this
        // item's object references as it leaves the hub, but the row
        // survives and still points at them. Pinning them back, as a
        // queue row pins a queued body, keeps a retained body out of the
        // collector; revive and purge release the pin.
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

    for (link, item) in &new.items {
        match old.items.get(link) {
            None => insert_item(conn, collection, link, item)?,
            Some(prev) => {
                if !item_columns_eq(prev, item) {
                    update_item(conn, collection, link, item)?;
                }
                save_bindings_diff(conn, collection, link, prev, item, superseded)?;
            }
        }
    }

    Ok(())
}

/// Whether two items' persisted columns, everything but their bindings,
/// match.
///
/// Every column `UPDATE_ITEM` writes has to be here: one left out can
/// never change again, since the diff reports the row unchanged and no
/// statement is issued for it.
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
    // NOTE: a retained row may still hold this primary key, the item
    // being back from a source or a client `add`. Reviving it in place
    // keeps its `seq`: a message holds one public id for life.
    if revive_item(conn, collection, link, item)? {
        return Ok(());
    }

    // NOTE: the public id is a property of the message, so a link id
    // already carrying a seq in another collection reuses it and every
    // placement shares one id; otherwise a fresh store-global id is
    // drawn, and ids are never reused.
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

/// Revives the retained row holding `(collection, link)`, if there is
/// one: it stops being retained (spec §11), adopts the incoming content
/// through the ordinary item update and binds the sources.
///
/// The retention pin the retire took is released here, and the caller's
/// refcount diff takes the live reference in the same transaction, so a
/// body kept only by the retained row is never sweepable in between.
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

/// Diffs one item's per-source bindings between `old` and `new`, issuing
/// only the binding writes that changed, and refusing the one write no
/// diff may express: a binding resolved to another handle (spec §10).
fn save_bindings_diff(
    conn: &Connection,
    collection: &str,
    link: &ReplicaLinkId,
    old: &ReplicaHubItem,
    new: &ReplicaHubItem,
    superseded: &BTreeSet<ReplicaHandle>,
) -> Result<(), PimdirError> {
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
            // NOTE: a handle-space rebuild superseded the handle this
            // binding holds, so the row is replaced rather than
            // repointed, the way spec §10 says a legitimate rebind goes.
            // `UPDATE_BINDING` could not do it in any case: it writes
            // every column but `handle`, for the reason below.
            Some(prev) if binding.handle != prev.handle && superseded.contains(&prev.handle) => {
                conn.execute(
                    sql::DELETE_BINDING,
                    named_params! { ":collection": collection, ":link_id": link.0, ":source": source.0 },
                )?;
                insert_binding(conn, collection, link, source, binding)?
            }
            // NOTE: a binding pins one handle, and repointing it would
            // destroy the evidence that a source holds an identity twice,
            // silently, at the write. The second copy has a key and an
            // item of its own now (spec §9), so refusing is a complete
            // answer and nothing is recorded in the incoming handle's
            // place. The engine mints before it writes, so this catches a
            // consumer staging its own writes, and a rebuilt handle space
            // handing two placements to one key.
            Some(prev) if binding.handle != prev.handle => {
                return Err(PimdirError::Rebind {
                    collection: collection.into(),
                    link_id: link.0.clone(),
                    source: source.0.clone(),
                    bound: prev.handle.0.clone(),
                    incoming: binding.handle.0.clone(),
                });
            }
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
            ":base_present": binding.base.is_some() as i64,
            ":conflicted": binding.conflicted as i64,
            ":conflict_revision": binding.conflicted.then_some(binding.conflict_revision.as_deref()).flatten(),
            ":conflict_object": conflict_object(binding).map(|hash| hash.0.as_str()),
            ":shared_object": binding.shared_object.as_ref().map(|hash| hash.0.as_str()),
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
            ":conflict_object": conflict_object(binding).map(|hash| hash.0.as_str()),
            ":shared_object": binding.shared_object.as_ref().map(|hash| hash.0.as_str()),
        },
    )?;
    Ok(())
}

/// The diverging remote body a binding is stuck on, as the column holds
/// it: the hash while the binding is conflicted, `NULL` otherwise.
///
/// Gated on the flag exactly as the revision beside it is (spec §13). A
/// body outliving the revision it was fetched at describes a version the
/// remote no longer holds, and it is also what releases the pin: a
/// resolved binding stops referencing the object, so the collector takes
/// it like any other unreferenced body.
fn conflict_object(binding: &ReplicaSourceBinding) -> Option<&ReplicaHash> {
    binding
        .conflicted
        .then_some(binding.conflict_object.as_ref())
        .flatten()
}

/// The multiset of object references a hub holds, keyed by hash: every
/// item's `object` and `conflict_object` plus every binding's
/// `base.object` and its own `conflict_object`. Computed in memory, so
/// refcount maintenance is a per-hash delta rather than a full-table
/// rescan.
///
/// A binding's `shared_object` is deliberately not among them, where the
/// column beside it is. It records which body this source last agreed
/// with and is only ever compared for equality, never read as bytes, and
/// a content hash compares the same after the body it named has been
/// swept. Counting it would pin every body a source ever agreed with for
/// as long as the binding lives, and buy nothing.
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
            // NOTE: the pin that keeps a diverging body readable until
            // someone resolves the conflict, which is an interval of
            // days. Read off the same gate the column is written
            // through, so the two can never disagree about what is
            // referenced.
            if let Some(conflict) = conflict_object(binding) {
                bump(conflict);
            }
        }
    }
    refs
}

/// Applies the change between two reference multisets as per-hash
/// refcount deltas (`refcount += new - old`), touching only hashes whose
/// count moved. A hash other collections reference keeps their share: the
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

/// Maps a client-read row to a [`PimdirItem`]. Shared by `list_items` and
/// `get_item`.
fn read_item_from_row(row: &Row) -> rusqlite::Result<PimdirItem> {
    let seq: i64 = row.get(0)?;
    let link: String = row.get(1)?;
    let flags: Option<String> = row.get(2)?;
    let object: Option<String> = row.get(3)?;
    let meta: Option<String> = row.get(4)?;
    let sort_key: String = row.get(5)?;
    let level: i64 = row.get(6)?;

    // NOTE: the retained page selects these seven columns and three more,
    // so one mapper reads both shapes; a live read stops at the level.
    let retention = match row.as_ref().column_count() > 7 {
        true => Some(PimdirRetention {
            at: row.get(7)?,
            by: row.get(8)?,
            size: row.get::<_, Option<i64>>(9)?.map(|size| size.max(0) as u64),
        }),
        false => None,
    };

    Ok(PimdirItem {
        seq,
        link_id: ReplicaLinkId(link),
        flags: codec::flags_from_json(flags.as_deref()),
        meta: meta.map(ReplicaMeta),
        sort_key,
        object: object.map(ReplicaHash),
        level: codec::level_from_int(level),
        retention,
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
    let conflict_object: Option<String> = row.get(9)?;
    let shared_object: Option<String> = row.get(10)?;

    // NOTE: either witness. The column is the fact, and a base of no
    // revision, no body and markers nobody has read is a real agreement
    // its three value columns cannot express: reading presence off them
    // alone has such a placement come back as never-agreed, so the sync
    // re-derives the same push every run. The value columns stay a
    // witness for a row written before the column existed.
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
            // NOTE: spec §13, the revision and the body beside it are
            // meaningful only while conflicted, so a resolved binding
            // cannot hand a stale pair to the next sync.
            conflict_revision: conflicted.then_some(conflict_revision).flatten(),
            conflict_object: conflicted
                .then_some(conflict_object)
                .flatten()
                .map(ReplicaHash),
            // NOTE: ungated, where the pair above is gated: the
            // agreement point is the ordinary state of an ordinary
            // binding, and the edit resolving a conflict needs the one
            // the conflict was filed at.
            shared_object: shared_object.map(ReplicaHash),
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

/// Writes every body a batch carries to the blob store, ahead of the
/// transaction that indexes them (spec §14).
///
/// A body is content-addressed and immutable, so writing it early can
/// only produce a file some later batch produces identically, and the
/// worst a crash between the two leaves is an orphan blob. Inside the
/// transaction the same write would hold SQLite's single writer lock
/// across a file write, two `fsync`s and a rename, serialising every
/// other writer behind an I/O path that touches no database page.
///
/// What keeps a collector out of the window this opens is the writer's
/// lock (spec §8), not the file's age: between the write and the commit
/// the file is on disk with no row, indistinguishable from an orphan.
///
/// [`write_blob`] is idempotent, so the batch may re-offer the same body
/// without cost, which lets [`apply_ops`] keep its own write as the floor
/// for a caller that cannot stage ahead.
fn stage_blobs(blobs: &Path, ops: &[ReplicaWriteOp]) -> io::Result<()> {
    for op in ops {
        if let ReplicaWriteOp::StoreObject {
            object,
            body: Some(body),
        } = op
        {
            write_blob(blobs, &object.hash.0, body)?;
        }
    }

    Ok(())
}

/// Writes a blob atomically (temp → `fsync` → rename → `fsync` the shard
/// directory, spec §5); a present hash is immutable and left untouched.
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
/// Syncing the file makes its bytes durable and says nothing about the
/// name that reaches them. The database commit is durable, so without
/// this a crash can leave a committed row pointing at a body that never
/// arrived: the one asymmetry the write order exists to prevent, the
/// reverse leaving at worst an orphan blob.
fn sync_dir(dir: &Path) -> io::Result<()> {
    fs::File::open(dir)?.sync_all()
}

/// Everything that can go wrong servicing the seam.
#[derive(Debug)]
pub enum PimdirError {
    /// The SQLite index refused a statement, or the connection itself failed.
    Sql(rusqlite::Error),
    /// The blob directory refused a read, a write or a rename.
    Io(io::Error),
    /// JSON encoding failed at the storage seam, the link id array a
    /// lookup hands to SQLite; a malformed queue payload reports as
    /// `Action`.
    Json(serde_json::Error),
    /// A queue action payload is malformed or unsupported (spec §15.3).
    Action(PimdirActionError),
    /// A write resolved an existing `(collection, link_id, source)`
    /// binding to a different handle, and was refused (spec §10).
    ///
    /// A binding pins one handle, so applying it would repoint the
    /// binding from the copy it held to another, which is where the
    /// evidence of a source holding one identity twice used to die. The
    /// second copy is an item of its own under a minted key (spec §9),
    /// which is what makes refusing a complete answer: nothing is
    /// recorded in the incoming handle's place. The one licensed rebind
    /// is the handle-space rebuild (spec §12), whose `Superseded` drop
    /// names the handle it replaces.
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
    /// The store's schema version is not one this crate services: it was
    /// written by a newer crate, or by a draft this one no longer reads.
    /// Such a store is recreated, never migrated.
    Version {
        /// The store's `user_version`.
        found: i64,
    },
    /// The store has no schema yet, and this opener does not create one.
    /// A producer and a reader both require the owner to have opened it
    /// first, which is the write that creates the database.
    Uncreated,
    /// The store's two schema stamps disagree, which spec §4.2 defines as
    /// corruption: `PRAGMA user_version` and `store_meta.version` mirror
    /// one another, so a store where they differ was half-written.
    VersionMismatch {
        /// The store's `PRAGMA user_version`.
        user_version: i64,
        /// The version its `store_meta` row records.
        store_meta: i64,
    },
    /// The store was created by a draft whose foreign keys lack the
    /// `ON UPDATE CASCADE` a rename depends on (spec §14), which no
    /// `ALTER TABLE` can add. Spec §6's other branch applies: the
    /// operator recreates the store, a resync of a derived cache.
    Unreconcilable {
        /// The first table found without the cascade.
        table: &'static str,
    },
    /// The store's `store_meta.hash_algo` is not one this crate computes,
    /// or not the one the caller declared. Either way the handle would
    /// name bodies the store does not use, so it is refused (spec §5).
    HashAlgo {
        /// The algorithm the store records.
        found: String,
        /// The algorithm the caller declared, when it declared one.
        declared: Option<&'static str>,
    },
    /// Another writer holds the store's single write lock (§8); the
    /// caller retries once that writer is done.
    Busy,
    /// Another process owns the store (§8), which this one asked to own
    /// too. Reported as soon as the lock is refused rather than waited
    /// out: a wait long enough to outlast a sync is a stall with no
    /// signal, and the caller is the only layer that can choose between
    /// retrying, backing off, queueing the intent and telling the user.
    Owned(PathBuf),
    /// A producer is between its blob write and the enqueue that pins it
    /// (§8), so a collector cannot run: the body it just wrote is
    /// referenced by nothing yet. Reported rather than waited out, since
    /// a producer holds its lock for as long as its handle lives.
    Staging(PathBuf),
}

impl fmt::Display for PimdirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PimdirError::Sql(err) => write!(f, "pimdir SQL error: {err}"),
            PimdirError::Io(err) => write!(f, "pimdir I/O error: {err}"),
            PimdirError::Json(err) => write!(f, "pimdir JSON error: {err}"),
            PimdirError::Action(err) => write!(f, "pimdir action error: {err}"),
            PimdirError::Rebind {
                collection,
                link_id,
                source,
                bound,
                incoming,
            } => write!(
                f,
                "pimdir binding {collection}/{link_id} on source {source} holds handle {bound}, and this write carries {incoming}: a binding pins one handle, and a second copy of an identity is stored under a key of its own"
            ),
            PimdirError::Version { found } => write!(
                f,
                "pimdir store schema version {found} is unsupported (this crate services version {})",
                sql::VERSION
            ),
            PimdirError::Uncreated => write!(
                f,
                "pimdir store has no schema yet: its owner has to create it first"
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
            PimdirError::Owned(store) => write!(
                f,
                "pimdir store at {} is owned by another process",
                store.display()
            ),
            PimdirError::Staging(store) => write!(
                f,
                "pimdir store at {} has a producer staging a body",
                store.display()
            ),
            PimdirError::Busy => write!(
                f,
                "pimdir store is busy: another writer holds the write lock; retry once it releases"
            ),
        }
    }
}

/// Maps a SQLite busy/locked failure to [`PimdirError::Busy`], leaving
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
