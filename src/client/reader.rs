//! [`PimdirReader`]: the read role (spec §8), and the pending-action
//! overlay a frontend reads through (spec §15.4).
//!
//! A store has one owner, any number of producers, and any number of
//! readers. The first two are handles that write; this one is not, and
//! says so by carrying no write at all rather than by a connection
//! refusing one. [`PimdirStore`](crate::client::PimdirStore) holds one and dereferences to it, so
//! the projection is a single implementation whichever role reads it.
//!
//! A reader built with [`with_pending`] folds the queue's pending
//! actions over the committed items before returning them, which is what
//! makes a frontend's own staged write visible before the owner applies
//! it. The fold covers the actions that address an existing item; a
//! queued create has no public id yet and is reported apart, by
//! [`pending_creates`].
//!
//! [`with_pending`]: PimdirReader::with_pending
//! [`pending_creates`]: PimdirReader::pending_creates

use alloc::{
    string::{String, ToString},
    vec::Vec,
};
use core::cmp::Ordering;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use io_replica::{
    collection::ReplicaCollectionId,
    hub::{ReplicaSourceBinding, ReplicaSourceId},
    object::ReplicaHash,
    placement::{ReplicaLevel, ReplicaLinkId},
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, named_params};

use crate::{
    client::{
        PimdirBlobs, PimdirCollection, PimdirConflict, PimdirError, PimdirItem, PimdirParkedAction,
        PimdirPendingAction, PimdirPlacement, binding_from_row, check_rename_cascades,
        check_version_agreement, collection_row, conflict_row, load_pending_actions,
        read_hash_algo, read_item_from_row, rows,
    },
    codec::{self, PimdirAction},
    hash::{PimdirHashAlgo, PimdirHasher},
    sql,
};

/// A pimdir store opened to read: the projection every role shares, and
/// no way to write it.
///
/// A reader owns nothing and takes no lock (spec §8), so any number of
/// them run against a store an owner is syncing, and none of them waits.
/// [`PimdirStore`](crate::client::PimdirStore) dereferences to one, which is what keeps the owner's
/// reads and a frontend's the same reads.
///
/// Built with [`with_pending`](Self::with_pending) it also overlays the
/// queue (spec §15.4), so a producer sees what it staged before the
/// owner applies it.
pub struct PimdirReader {
    pub(super) conn: Connection,
    /// The store directory, which the collector locks and the blob tree hangs
    /// off.
    pub(super) dir: PathBuf,
    pub(super) blobs: PathBuf,
    /// The hash this store names its objects by (spec §5), read back from
    /// `store_meta.hash_algo` so every body a consumer hashes lands under
    /// the name the store already uses.
    pub(super) hash: PimdirHashAlgo,
    /// Whether the item reads fold the pending queue over the committed
    /// rows (spec §15.4). Chosen when the reader is built, never per
    /// call, so one handle cannot answer two ways about one collection.
    overlay: bool,
}

impl PimdirReader {
    /// Opens an **existing** store rooted at `dir` to read.
    ///
    /// The database is opened with `SQLITE_OPEN_READ_ONLY`: nothing is
    /// created, so a missing database errors, one no owner has stamped
    /// yet is [`PimdirError::Uncreated`], and any other schema version is
    /// refused with [`PimdirError::Version`].
    ///
    /// No lock is taken, so this never waits on a sync in flight and
    /// never keeps one out.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, PimdirError> {
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

        Ok(Self::over(
            conn,
            dir.to_path_buf(),
            dir.join("objects"),
            hash,
        ))
    }

    /// Wraps an already-opened connection, the constructor
    /// [`PimdirStore`] builds its own reader through.
    pub(super) fn over(
        conn: Connection,
        dir: PathBuf,
        blobs: PathBuf,
        hash: PimdirHashAlgo,
    ) -> Self {
        Self {
            conn,
            dir,
            blobs,
            hash,
            overlay: false,
        }
    }

    /// Reads through the queue's pending actions as well as the committed
    /// rows (spec §15.4), so an action this process staged is visible
    /// before the store's owner applies it.
    ///
    /// The fold covers the actions that address an existing item:
    /// `set-flags` and `update` restate it, `remove` and `move` take it
    /// out of the collection, and `move` and `copy` bring it into the
    /// target. All of them keep the item's public id, a `seq` following
    /// the link id store-wide (spec §9.1), so nothing here invents an
    /// identifier.
    ///
    /// A queued create is not folded in: it has no `seq` until the owner
    /// applies it, and it is a request to create an item rather than one.
    /// [`pending_creates`](Self::pending_creates) reports those.
    ///
    /// A parked row is never folded either: its error says it will not be
    /// applied without an operator, and reading it as pending would
    /// promise otherwise.
    ///
    /// A page keeps its meaning: it comes back short only where the
    /// collection ends, staged removals or not, so a caller pages the way
    /// it always did. The cost is that a read consults the queue, which is
    /// a handful of small statements over rows a sync drains, not a scan.
    pub fn with_pending(mut self) -> Self {
        self.overlay = true;
        self
    }

    /// Whether this reader folds the pending queue over its item reads.
    pub fn overlays_pending(&self) -> bool {
        self.overlay
    }
}

/// The client read surface: what a consumer projects into an envelope, a
/// vCard or an event, and the queue and generation reads beside it.
impl PimdirReader {
    /// The hash this store names its objects by (spec §5).
    pub fn hash_algo(&self) -> PimdirHashAlgo {
        self.hash
    }

    /// A blob handle over this store's object directory, bound to the hash the
    /// store names its bodies by.
    ///
    /// Independent of the SQLite connection, so a body can be read while
    /// the store is mutably borrowed servicing a sync.
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

    /// An incremental hasher for a body streamed into the blob store
    /// rather than held whole in memory, paired with
    /// [`PimdirBlobs::writer`].
    pub fn hasher(&self) -> PimdirHasher {
        self.hash.hasher()
    }

    /// The account a collection is grouped under.
    ///
    /// The outer `Option` is whether the collection exists, the inner one
    /// whether it is grouped: `Ok(None)` for an unknown collection,
    /// `Ok(Some(None))` for one in a single-account store.
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

    /// The declared media type of a collection, or `None` if the store
    /// has never seen it. An empty string means the collection exists but
    /// was created lazily by a sync before any
    /// [`ensure_collection`](crate::client::PimdirStore::ensure_collection) declared its kind.
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
    /// Ordered by `sort_order` then `id`, unordered collections last. A
    /// direct getter: it observes the shared truth and never mutates, and
    /// writes go through io-replica's [`write`](io_replica::client::ReplicaStorage::write).
    pub fn list_collections(&self) -> Result<Vec<PimdirCollection>, PimdirError> {
        Ok(rows(&self.conn, sql::LIST_COLLECTIONS, [], collection_row)?)
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
        Ok(rows(
            &self.conn,
            sql::LIST_COLLECTIONS_BY_ACCOUNT,
            named_params! { ":account": account },
            collection_row,
        )?)
    }

    /// The accounts owning at least one collection.
    ///
    /// Not a configured roster: a store learns an account only through
    /// its collections (spec §9.2), so one with none yet does not appear
    /// here and a consumer holding the real roster reads its own config.
    pub fn list_accounts(&self) -> Result<Vec<String>, PimdirError> {
        Ok(rows(&self.conn, sql::LIST_ACCOUNTS, [], |r| r.get(0))?)
    }

    /// Every live placement of one identity, with the collection and account it
    /// sits in (spec §9.2).
    ///
    /// The store reports where a link id occurs and takes no position on
    /// whether the placements are one thing. A mail view lists them, two
    /// receipts of a newsletter having two read states; a contact view
    /// may offer to merge them. Both read these rows.
    pub fn link_placements(&self, link_id: &str) -> Result<Vec<PimdirPlacement>, PimdirError> {
        Ok(rows(
            &self.conn,
            sql::LIST_LINK_PLACEMENTS,
            named_params! { ":link_id": link_id },
            |r| {
                Ok(PimdirPlacement {
                    collection: r.get(0)?,
                    account: r.get(1)?,
                    seq: r.get(2)?,
                    link_id: ReplicaLinkId(link_id.to_string()),
                    object: r.get::<_, Option<String>>(3)?.map(ReplicaHash),
                    flags: codec::flags_from_json(r.get::<_, Option<String>>(4)?.as_deref()),
                    level: codec::level_from_int(r.get(5)?),
                })
            },
        )?)
    }

    /// Every live placement of one body, by content hash: the dedup axis
    /// rather than the identity one, so it pairs placements two servers
    /// gave different link ids.
    pub fn object_placements(&self, hash: &str) -> Result<Vec<PimdirPlacement>, PimdirError> {
        Ok(rows(
            &self.conn,
            sql::LIST_OBJECT_PLACEMENTS,
            named_params! { ":hash": hash },
            |r| {
                Ok(PimdirPlacement {
                    collection: r.get(0)?,
                    account: r.get(1)?,
                    seq: r.get(2)?,
                    link_id: ReplicaLinkId(r.get(3)?),
                    object: Some(ReplicaHash(hash.to_string())),
                    flags: codec::flags_from_json(r.get::<_, Option<String>>(4)?.as_deref()),
                    level: codec::level_from_int(r.get(5)?),
                })
            },
        )?)
    }

    /// A keyset page of a collection's live items (client read surface).
    ///
    /// `after` is the exclusive lower bound on `link_id`, `None` starting
    /// from the beginning; at most `limit` items come back ordered by
    /// `link_id`, so the last item's [`link_id`](PimdirItem::link_id) is
    /// the next page's cursor. Tombstones are excluded, and each item
    /// carries its `level`, so a body's absence shows without probing the
    /// blobs.
    pub fn list_items(
        &self,
        collection: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<PimdirItem>, PimdirError> {
        let after = after.unwrap_or("");
        self.overlaid(
            collection,
            limit,
            |limit| {
                Ok(rows(
                    &self.conn,
                    sql::LIST_ITEMS_PAGE,
                    named_params! {
                        ":collection": collection,
                        ":after": after,
                        ":limit": limit as i64,
                    },
                    read_item_from_row,
                )?)
            },
            |item| item.link_id.0.as_str() > after,
            |left, right| left.link_id.0.cmp(&right.link_id.0),
        )
    }

    /// A keyset page of a collection's live items in the kind's own
    /// ascending order (spec §9.3): A to Z for contacts, earliest first
    /// for mail and calendars.
    ///
    /// `after` is the previous page's last `(sort_key, seq)`, `None`
    /// starting from the beginning. The pair is the cursor because a sort
    /// key is not unique and `seq`, unique per collection, is what makes
    /// the page total: no item is skipped or repeated across a boundary.
    pub fn list_items_page_asc(
        &self,
        collection: &str,
        after: Option<(&str, i64)>,
        limit: usize,
    ) -> Result<Vec<PimdirItem>, PimdirError> {
        // NOTE: no real key sorts before an unknown one ascending, so the
        // empty string with seq 0 is the true beginning, not a sentinel.
        let (key, seq) = after.unwrap_or(("", 0));
        self.sorted_page(
            sql::LIST_ITEMS_PAGE_ASC,
            collection,
            Some((key, seq)),
            limit,
            false,
        )
    }

    /// The same page descending: newest first for mail and calendars, Z
    /// to A for contacts.
    ///
    /// `None` starts from the end, which the statement expresses by
    /// binding a key above every representable one, so a caller never
    /// invents that sentinel itself.
    pub fn list_items_page_desc(
        &self,
        collection: &str,
        after: Option<(&str, i64)>,
        limit: usize,
    ) -> Result<Vec<PimdirItem>, PimdirError> {
        self.sorted_page(sql::LIST_ITEMS_PAGE_DESC, collection, after, limit, true)
    }

    fn sorted_page(
        &self,
        statement: &str,
        collection: &str,
        after: Option<(&str, i64)>,
        limit: usize,
        descending: bool,
    ) -> Result<Vec<PimdirItem>, PimdirError> {
        let after = after.map(|(key, seq)| (key.to_string(), seq));
        self.overlaid(
            collection,
            limit,
            |limit| {
                Ok(rows(
                    &self.conn,
                    statement,
                    named_params! {
                        ":collection": collection,
                        ":after_key": after.as_ref().map(|(key, _)| key.as_str()),
                        ":after_seq": after.as_ref().map(|(_, seq)| *seq).unwrap_or_default(),
                        ":limit": limit as i64,
                    },
                    read_item_from_row,
                )?)
            },
            |item| {
                let here = (item.sort_key.as_str(), item.seq);
                match &after {
                    None => true,
                    Some((key, seq)) if descending => here < (key.as_str(), *seq),
                    Some((key, seq)) => here > (key.as_str(), *seq),
                }
            },
            |left, right| {
                let order =
                    (left.sort_key.as_str(), left.seq).cmp(&(right.sort_key.as_str(), right.seq));
                if descending { order.reverse() } else { order }
            },
        )
    }

    /// One live item by its public id `(collection, seq)`, or `None`. A
    /// tombstoned item reads as `None`, and the returned item carries its
    /// internal `link_id` for the caller to edit by.
    pub fn get_item(&self, collection: &str, seq: i64) -> Result<Option<PimdirItem>, PimdirError> {
        let item = self.committed_item(collection, seq)?;
        if !self.overlay {
            return Ok(item);
        }

        let pending = self.pending(collection)?;
        let item = match item {
            Some(item) => Some(item),
            // NOTE: absent here and arriving there is one item, not none:
            // a staged move or copy is read from the collection whose row
            // still holds it.
            None => match pending.arrivals.get(&seq) {
                Some(from) => self.committed_item(from, seq)?,
                None => None,
            },
        };
        Ok(item.and_then(|item| fold(item, pending.edits.get(&seq))))
    }

    /// Resolves an item's public id (`seq`) from its internal `link_id`,
    /// the inverse of [`get_item`](Self::get_item), for a consumer that
    /// just staged an add and wants the id it now shows under.
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

    /// Every source's binding of one item, keyed by source: the handle it is
    /// bound to, the base the last sync agreed on, and the conflict its own
    /// sync is stuck on (spec §13).
    ///
    /// The same shape a hub carries per item, read for one item rather than a
    /// collection: an operator asking why a placement stopped moving is asking
    /// about exactly these columns, and nothing else exposes them.
    pub fn item_bindings(
        &self,
        collection: &str,
        link_id: &str,
    ) -> Result<BTreeMap<ReplicaSourceId, ReplicaSourceBinding>, PimdirError> {
        Ok(rows(
            &self.conn,
            sql::ITEM_BINDINGS,
            named_params! { ":collection": collection, ":link_id": link_id },
            binding_from_row,
        )?
        .into_iter()
        .map(|(_, source, binding)| (source, binding))
        .collect())
    }

    /// The bindings waiting for a decision, across one account's
    /// collections, ordered by collection then link id then source.
    ///
    /// `None` lists a single-account store whole, the account grouping
    /// nothing there. Each row carries the three bodies the divergence
    /// is between, so a resolver holding no credentials reads base,
    /// local and remote from the store alone (spec §13).
    ///
    /// The question a sync answers at the end of every run, and the one
    /// a listing command asks directly. Both are served by the partial
    /// index over the flag, so a store with nothing outstanding pays for
    /// an empty index rather than for a pass over every collection.
    pub fn list_conflicts(
        &self,
        account: Option<&str>,
    ) -> Result<Vec<PimdirConflict>, PimdirError> {
        Ok(rows(
            &self.conn,
            sql::LIST_CONFLICTED_BINDINGS,
            named_params! { ":account": account },
            conflict_row,
        )?)
    }

    /// The distinct source names the store has synced against, across all
    /// collections. A client attributes its writes with this: a store
    /// synced as a single source has exactly one, so the app writes as it
    /// without configuration.
    pub fn distinct_sources(&self) -> Result<Vec<String>, PimdirError> {
        Ok(rows(&self.conn, sql::LIST_SOURCES, [], |r| r.get(0))?)
    }

    /// A collection's live (non-tombstone) item count (client read surface).
    pub fn count_items(&self, collection: &str) -> Result<u64, PimdirError> {
        let count: i64 = self.conn.query_row(
            sql::COUNT_ITEMS,
            named_params! { ":collection": collection },
            |r| r.get(0),
        )?;
        let mut count = count.max(0) as u64;
        if !self.overlay {
            return Ok(count);
        }

        let pending = self.pending(collection)?;
        for (seq, edits) in &pending.edits {
            let Some(item) = self.committed_item(collection, *seq)? else {
                continue;
            };
            if fold(item, Some(edits)).is_none() {
                count -= 1;
            }
        }
        Ok(count + self.arrived(&pending)?.len() as u64)
    }

    /// A keyset page of a collection's retained items.
    ///
    /// `after` is the exclusive lower bound on the public `seq`, `None`
    /// starting from the beginning; at most `limit` items come back
    /// ordered by `seq`, so the last item's [`seq`](PimdirItem::seq) is
    /// the next page's cursor. The only read that returns retained items:
    /// a caller presents them as a trash view, never merged into the live
    /// listing.
    pub fn list_retained(
        &self,
        collection: &ReplicaCollectionId,
        after: Option<i64>,
        limit: usize,
    ) -> Result<Vec<PimdirItem>, PimdirError> {
        Ok(rows(
            &self.conn,
            sql::LIST_RETAINED_PAGE,
            named_params! {
                ":collection": collection.0,
                ":after": after.unwrap_or(0),
                ":limit": limit as i64,
            },
            read_item_from_row,
        )?)
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
    /// An upper bound on what a purge would reclaim: a body a live item
    /// also points at keeps that reference and survives the sweep.
    /// Reported so an operator can price a retention duration.
    pub fn retained_bytes(&self) -> Result<u64, PimdirError> {
        let bytes: i64 = self.conn.query_row(sql::RETAINED_BYTES, [], |r| r.get(0))?;
        Ok(bytes.max(0) as u64)
    }

    /// A collection's handle-space epoch (spec §12), or `None` when the
    /// store has never seen it. Starts at 1, bumped only by
    /// [`write_rekeyed`](crate::client::PimdirSourceStore::write_rekeyed), so a frontend
    /// derives an IMAP UIDVALIDITY from it alone.
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
        Ok(rows(&self.conn, sql::LIST_QUEUED_COLLECTIONS, [], |r| {
            r.get(0)
        })?)
    }

    /// A collection's pending (non-parked) actions in append order,
    /// decoded (spec §15.4): a frontend overlays them on its item
    /// projection for read-your-writes. An undecodable payload errors,
    /// and the owner's next drain parks such a row.
    pub fn pending_actions(
        &self,
        collection: &str,
    ) -> Result<Vec<PimdirPendingAction>, PimdirError> {
        load_pending_actions(&self.conn, collection)
    }

    /// Every parked action across the store, in append order, for status
    /// surfaces and operator repair. Parked rows are skipped by the drain
    /// and never silently deleted.
    pub fn parked_actions(&self) -> Result<Vec<PimdirParkedAction>, PimdirError> {
        Ok(rows(&self.conn, sql::LOAD_PARKED_ACTIONS, [], |r| {
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
        })?)
    }
}

/// What the queue's pending actions change about one collection (spec
/// §15.4): the raw material of the overlay, folded once per read.
///
/// Built from the whole pending queue rather than one collection's rows,
/// because a `move` or a `copy` is enqueued against the collection the
/// item leaves and names the one it enters, so what arrives in a
/// collection is written down elsewhere.
#[derive(Debug, Default)]
struct PimdirPending {
    /// Actions restating or removing an item the collection already
    /// holds, by public id, in append order.
    edits: BTreeMap<i64, Vec<PimdirAction>>,
    /// Items another collection's pending `move` or `copy` brings in,
    /// each mapped to the collection its row is still read from.
    arrivals: BTreeMap<i64, String>,
    /// Queued creates targeting the collection. Counted rather than
    /// listed: a create has no public id until the owner applies it.
    creates: usize,
}

impl PimdirPending {
    /// How many rows the fold can drop from a page: the items an action
    /// takes out of the collection, by removing them or by moving them
    /// away.
    fn removals(&self) -> usize {
        self.edits
            .values()
            .filter(|actions| {
                actions
                    .iter()
                    .any(|action| matches!(action, PimdirAction::Remove { .. }))
            })
            .count()
    }
}

impl PimdirReader {
    /// One live item as the committed rows hold it, the queue ignored.
    fn committed_item(
        &self,
        collection: &str,
        seq: i64,
    ) -> Result<Option<PimdirItem>, PimdirError> {
        Ok(self
            .conn
            .query_row(
                sql::GET_ITEM,
                named_params! { ":collection": collection, ":seq": seq },
                read_item_from_row,
            )
            .optional()?)
    }

    /// Folds the store's pending queue into what it changes about one
    /// collection.
    ///
    /// The rows are walked in global append order, which is what makes a
    /// later action win over an earlier one on the same item whichever
    /// collection each was enqueued against.
    fn pending(&self, collection: &str) -> Result<PimdirPending, PimdirError> {
        let mut queued = Vec::new();
        for from in self.queued_collections()? {
            for action in load_pending_actions(&self.conn, &from)? {
                queued.push((from.clone(), action));
            }
        }
        queued.sort_by_key(|(_, action)| action.id);

        let mut pending = PimdirPending::default();
        for (from, action) in queued {
            let here = from == collection;
            match &action.action {
                PimdirAction::Add { .. } if here => pending.creates += 1,
                PimdirAction::SetFlags { seq, .. }
                | PimdirAction::Update { seq, .. }
                | PimdirAction::Remove { seq }
                    if here =>
                {
                    pending.edits.entry(*seq).or_default().push(action.action);
                }
                PimdirAction::Move { seq, to } => {
                    if here && to.0 != collection {
                        pending
                            .edits
                            .entry(*seq)
                            .or_default()
                            .push(PimdirAction::Remove { seq: *seq });
                    }
                    if !here && to.0 == collection {
                        pending.arrivals.insert(*seq, from);
                    }
                }
                PimdirAction::Copy { seq, to } if !here && to.0 == collection => {
                    pending.arrivals.insert(*seq, from);
                }
                _ => {}
            }
        }
        Ok(pending)
    }

    /// The items pending moves and copies bring into the collection,
    /// read from where their rows still sit and folded like any other.
    fn arrived(&self, pending: &PimdirPending) -> Result<Vec<PimdirItem>, PimdirError> {
        let mut items = Vec::new();
        for (seq, from) in &pending.arrivals {
            let Some(item) = self.committed_item(from, *seq)? else {
                continue;
            };
            if let Some(item) = fold(item, pending.edits.get(seq)) {
                items.push(item);
            }
        }
        Ok(items)
    }

    /// Folds the overlay into one page: the committed items restated or
    /// dropped, the arrivals that fall inside the page's window merged
    /// in, and the whole re-ordered and cut back to the limit.
    ///
    /// Cutting after the merge is what keeps paging total: the page's
    /// last item is still the next page's cursor, and an arrival past it
    /// comes back on that next page rather than being lost here.
    fn overlaid(
        &self,
        collection: &str,
        limit: usize,
        fetch: impl Fn(usize) -> Result<Vec<PimdirItem>, PimdirError>,
        inside: impl Fn(&PimdirItem) -> bool,
        order: impl Fn(&PimdirItem, &PimdirItem) -> Ordering,
    ) -> Result<Vec<PimdirItem>, PimdirError> {
        if !self.overlay {
            return fetch(limit);
        }

        let pending = self.pending(collection)?;
        // NOTE: a staged removal drops a row the statement returned, so a
        // page asked for exactly `limit` rows would come back short in the
        // middle of a collection, and a caller paging until a short page
        // stops early and never sees the rest. At most one row per removing
        // action can go, so over-reading by that many makes a page short
        // only where the collection really ends.
        let page = fetch(limit + pending.removals())?;
        let mut items: Vec<PimdirItem> = page
            .into_iter()
            .filter_map(|item| {
                let edits = pending.edits.get(&item.seq);
                fold(item, edits)
            })
            .collect();

        for item in self.arrived(&pending)? {
            if inside(&item) && !items.iter().any(|held| held.seq == item.seq) {
                items.push(item);
            }
        }

        items.sort_by(order);
        items.truncate(limit);
        Ok(items)
    }

    /// The queued creates targeting a collection, in append order (spec
    /// §15.4).
    ///
    /// Reported apart from the items because a create has no public id
    /// until the owner applies it, so there is nothing to address it by
    /// and no envelope to put it in. A consumer surfaces them its own
    /// way: a count under a listing, a queue view of its own, or the
    /// operator CLI's.
    pub fn pending_creates(
        &self,
        collection: &str,
    ) -> Result<Vec<PimdirPendingAction>, PimdirError> {
        Ok(self
            .pending_actions(collection)?
            .into_iter()
            .filter(|queued| matches!(queued.action, PimdirAction::Add { .. }))
            .collect())
    }

    /// How many creates the collection has queued, the count a listing
    /// reports so a staged item reads as queued rather than as lost.
    pub fn count_pending_creates(&self, collection: &str) -> Result<usize, PimdirError> {
        Ok(self.pending_creates(collection)?.len())
    }
}

/// Folds an item's pending actions into it, `None` when they take it out
/// of the collection.
///
/// `set-flags` is absolute rather than a delta (spec §15.3), so the last
/// one wins outright; `update` repoints the body, which a producer wrote
/// before enqueueing, so the item reads as `Full`.
fn fold(mut item: PimdirItem, actions: Option<&Vec<PimdirAction>>) -> Option<PimdirItem> {
    for action in actions.into_iter().flatten() {
        match action {
            PimdirAction::SetFlags { flags, .. } => item.flags = flags.clone(),
            PimdirAction::Update { object, meta, .. } => {
                item.object = Some(object.clone());
                if meta.is_some() {
                    item.meta = meta.clone();
                }
                item.level = ReplicaLevel::Full;
            }
            PimdirAction::Remove { .. } => return None,
            _ => {}
        }
    }
    Some(item)
}
