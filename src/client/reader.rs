//! # The reader
//!
//! The read role (STORAGE §8, §14.1): a handle that takes no lock and
//! carries no write, whose projection the owner shares by dereferencing
//! to it. Built with [`with_pending`] it folds the queue's pending
//! actions over the committed rows (§15.4), so a producer sees what it
//! staged before the owner applies it.
//!
//! [`with_pending`]: PimdirReader::with_pending

use core::cmp::Ordering;

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use rusqlite::{Connection, OpenFlags, OptionalExtension, Row, named_params};

use crate::{
    client::{
        PimdirError,
        blobs::PimdirBlobs,
        producer::{PimdirParkedAction, PimdirPendingAction, pending_actions},
        rows, schema,
        write::{
            PimdirSummaryTable, binding_from_row, kind_of, load_addresses, load_summaries,
            tables_of,
        },
    },
    codec::{self, PimdirAction},
    collection::PimdirCollectionId,
    hash::{PimdirHashAlgo, PimdirHasher},
    hub::{PimdirBinding, PimdirSourceId},
    object::PimdirHash,
    placement::{PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId},
    sql,
    summary::{PimdirAddressRole, PimdirSummary},
};

/// A pimdir store opened to read: the projection every role shares.
pub struct PimdirReader {
    pub(crate) conn: Connection,
    /// The store directory, which the collector locks and the blobs hang off.
    pub(crate) dir: PathBuf,
    /// The hash this store names its objects by (§5).
    pub(crate) hash: PimdirHashAlgo,
    /// Whether item reads fold the pending queue over the rows (§15.4).
    overlay: bool,
}

/// A collection as a read reports it, kind-agnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirCollection {
    /// The stable collection id.
    pub id: String,
    /// The account it is grouped under (§9.2), `None` in a single-account store.
    pub account: Option<String>,
    /// The declared media type, or the empty string when never declared.
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
    /// The handle-space epoch (§12), starting at 1.
    pub generation: i64,
}

/// One live item as a read reports it, its summary joined when the read
/// carries one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirItem {
    /// The public id (§9.1), the same in every collection the item is in.
    pub seq: i64,
    /// The item's key, internal: a consumer reads and edits by `seq`.
    pub link_id: PimdirLinkId,
    /// The flag set.
    pub flags: PimdirFlags,
    /// The kind's ordering key (§9.3); empty means unknown.
    pub sort_key: String,
    /// The body's hash; `None` until hydrated.
    pub object: Option<PimdirHash>,
    /// The detail tier reached.
    pub level: PimdirLevel,
    /// The summary and addresses (Annex A) on the reads that join them.
    pub summary: Option<PimdirSummary>,
    /// What retention holds about the row, `None` while it is live.
    pub retention: Option<PimdirRetention>,
}

/// What retention holds about an item (§11), on the trash view's rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirRetention {
    /// The RFC 3339 instant the last binding vanished.
    pub at: String,
    /// The source whose removal retired the item; diagnostic.
    pub by: Option<String>,
    /// The body's size in bytes, `None` alongside an absent body.
    pub size: Option<u64>,
}

/// Where one identity or one body sits (§9.2): one live placement with
/// the collection and account it occurs in. A fact, not a verdict.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirItemLocation {
    /// The collection the placement sits in.
    pub collection: String,
    /// The account that collection is grouped under.
    pub account: Option<String>,
    /// The item's public id.
    pub seq: i64,
    /// The item's key.
    pub link_id: PimdirLinkId,
    /// The body the placement points at.
    pub object: Option<PimdirHash>,
    /// The flag set.
    pub flags: PimdirFlags,
    /// The detail tier reached.
    pub level: PimdirLevel,
}

/// One live placement naming an address (Annex A.6): the person axis.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirAddressPlacement {
    /// The canonical address matched.
    pub address: String,
    /// The role it plays for the item.
    pub role: PimdirAddressRole,
    /// The collection the item sits in.
    pub collection: String,
    /// The account that collection is grouped under.
    pub account: Option<String>,
    /// The collection's kind.
    pub kind: String,
    /// The item's public id.
    pub seq: i64,
    /// The item's sort key.
    pub sort_key: String,
}

/// One binding waiting for a decision (§13), with the three bodies a
/// resolver merges: the base, the item's own, and the remote's.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirConflict {
    /// The collection the binding sits in.
    pub collection: String,
    /// The item's key.
    pub link_id: PimdirLinkId,
    /// The source that diverged from its own remote.
    pub source: PimdirSourceId,
    /// The item's handle on that source.
    pub handle: PimdirHandle,
    /// The remote revision observed when the divergence was recorded.
    pub conflict_revision: Option<String>,
    /// The body the last sync agreed on.
    pub base_object: Option<PimdirHash>,
    /// The local side, the item's own body.
    pub object: Option<PimdirHash>,
    /// The remote side, `None` until the upgrade supplies it.
    pub conflict_object: Option<PimdirHash>,
}

/// One item the change feed reports (§4.5).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirItemChange {
    /// The collection the item sits in.
    pub collection: String,
    /// The item's key.
    pub link_id: PimdirLinkId,
    /// The item's public id.
    pub seq: i64,
    /// The stamp the row took.
    pub changed: i64,
    /// Whether the item is deleted or retained.
    pub deleted: bool,
    /// The retention instant, when retained.
    pub retained_at: Option<String>,
}

/// One collection the change feed reports (§4.5).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirCollectionChange {
    /// The collection id, the new one after a rename.
    pub id: String,
    /// The account it is grouped under.
    pub account: Option<String>,
    /// The declared kind.
    pub kind: String,
    /// The display name.
    pub name: String,
    /// The stamp the row took.
    pub changed: i64,
}

/// The change feed's cursor (§4.5): the next stamp and the purge count.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PimdirChangeCursor {
    /// Every stamp below this one is drawn.
    pub next_change: i64,
    /// How many rows left without a stamp.
    pub purges: i64,
}

impl PimdirReader {
    /// Opens an existing store rooted at `dir` to read, refusing one no
    /// owner has created and one at another version. Takes no lock.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, PimdirError> {
        let dir = dir.as_ref();
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(dir.join("pimdir.db"), flags)?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 30000;")?;
        schema::check_version(&conn)?;
        let hash = schema::hash_algo(&conn, None)?;

        Ok(Self::over(conn, dir.to_path_buf(), hash))
    }

    /// Wraps an already-opened connection, the owner's own reader.
    pub(crate) fn over(conn: Connection, dir: PathBuf, hash: PimdirHashAlgo) -> Self {
        Self {
            conn,
            dir,
            hash,
            overlay: false,
        }
    }

    /// Reads through the queue's pending actions as well as the committed
    /// rows (§15.4). The fold covers the kinds addressing an existing
    /// item; a queued create is reported apart by
    /// [`pending_creates`](Self::pending_creates), a parked row never.
    pub fn with_pending(mut self) -> Self {
        self.overlay = true;
        self
    }

    /// Whether this reader folds the pending queue over its item reads.
    pub fn overlays_pending(&self) -> bool {
        self.overlay
    }

    /// The hash this store names its objects by (§5).
    pub fn hash_algo(&self) -> PimdirHashAlgo {
        self.hash
    }

    /// The blob directory, independent of the SQLite connection.
    pub fn blobs(&self) -> PimdirBlobs {
        PimdirBlobs::open(&self.dir, self.hash)
    }

    /// The content hash of a whole body, under this store's algorithm.
    pub fn hash(&self, bytes: &[u8]) -> PimdirHash {
        self.hash.hash(bytes)
    }

    /// An incremental hasher for a body streamed into the blob store.
    pub fn hasher(&self) -> PimdirHasher {
        self.hash.hasher()
    }
}

/// The collection reads.
impl PimdirReader {
    /// The account a collection is grouped under: `Ok(None)` for an
    /// unknown collection, `Ok(Some(None))` for an ungrouped one.
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

    /// The declared media type of a collection, `None` for an unknown one
    /// and empty for one a sync created before any declaration.
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

    /// Every collection, ordered by `sort_order` then id.
    pub fn list_collections(&self) -> Result<Vec<PimdirCollection>, PimdirError> {
        Ok(rows(
            &self.conn,
            sql::LIST_COLLECTIONS,
            [],
            collection_from_row,
        )?)
    }

    /// One account's collections (§9.2); `None` selects an ungrouped store's.
    pub fn list_collections_by_account(
        &self,
        account: Option<&str>,
    ) -> Result<Vec<PimdirCollection>, PimdirError> {
        Ok(rows(
            &self.conn,
            sql::LIST_COLLECTIONS_BY_ACCOUNT,
            named_params! { ":account": account },
            collection_from_row,
        )?)
    }

    /// The accounts owning at least one collection; not a configured roster.
    pub fn list_accounts(&self) -> Result<Vec<String>, PimdirError> {
        Ok(rows(&self.conn, sql::LIST_ACCOUNTS, [], |r| r.get(0))?)
    }

    /// A collection's handle-space epoch (§12), `None` for an unknown one.
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

    /// The distinct source names the store has synced against.
    pub fn distinct_sources(&self) -> Result<Vec<String>, PimdirError> {
        Ok(rows(&self.conn, sql::LIST_SOURCES, [], |r| r.get(0))?)
    }
}

/// The item reads, live only and keyed by the public id.
impl PimdirReader {
    /// A keyset page of live items in link-id order, the sweep that sees
    /// every item once; `after` is the exclusive lower bound.
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
                    item_from_row,
                )?)
            },
            |item| item.link_id.0.as_str() > after,
            |left, right| left.link_id.0.cmp(&right.link_id.0),
        )
    }

    /// A keyset page in the kind's ascending order (§9.3), cursor
    /// `(sort_key, seq)`, `None` starting from the beginning.
    pub fn list_items_page_asc(
        &self,
        collection: &str,
        after: Option<(&str, i64)>,
        limit: usize,
    ) -> Result<Vec<PimdirItem>, PimdirError> {
        let (key, seq) = after.unwrap_or(("", 0));
        self.sorted_page(
            sql::LIST_ITEMS_PAGE_ASC,
            collection,
            Some((key, seq)),
            limit,
            false,
        )
    }

    /// The same page descending, `None` starting from the end.
    pub fn list_items_page_desc(
        &self,
        collection: &str,
        after: Option<(&str, i64)>,
        limit: usize,
    ) -> Result<Vec<PimdirItem>, PimdirError> {
        self.sorted_page(sql::LIST_ITEMS_PAGE_DESC, collection, after, limit, true)
    }

    /// A page in the kind's natural direction with each item's summary
    /// and addresses joined (§14.1): newest first for mail, ascending
    /// for contacts and calendars, a calendar's three tables merged.
    ///
    /// A collection whose kind was never declared pages without summaries.
    pub fn list_summaries(
        &self,
        collection: &str,
        after: Option<(&str, i64)>,
        limit: usize,
    ) -> Result<Vec<PimdirItem>, PimdirError> {
        let kind = kind_of(&self.conn, collection)?;
        let tables = tables_of(&kind);
        let descending = kind.starts_with("message/rfc822");
        let statements: Vec<(&str, PimdirSummaryTable)> = match kind
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
        {
            "message/rfc822" => alloc::vec![(sql::LIST_MAIL_PAGE_DESC, PimdirSummaryTable::Mail)],
            "text/vcard" => alloc::vec![(sql::LIST_CONTACTS_PAGE_ASC, PimdirSummaryTable::Contact)],
            "text/calendar" => alloc::vec![
                (sql::LIST_EVENTS_PAGE_ASC, PimdirSummaryTable::Event),
                (sql::LIST_TASKS_PAGE_ASC, PimdirSummaryTable::Task),
                (sql::LIST_JOURNALS_PAGE_ASC, PimdirSummaryTable::Journal),
            ],
            _ => return self.list_items_page_asc(collection, after, limit),
        };

        let after = after.map(|(key, seq)| (key.to_string(), seq));
        let mut items = self.overlaid(
            collection,
            limit,
            |limit| {
                let mut page: Vec<PimdirItem> = Vec::new();
                for (statement, table) in &statements {
                    let mut rows = rows(
                        &self.conn,
                        statement,
                        named_params! {
                            ":collection": collection,
                            ":after_key": match (&after, descending) {
                                (None, true) => None,
                                (None, false) => Some(""),
                                (Some((key, _)), _) => Some(key.as_str()),
                            },
                            ":after_seq": after.as_ref().map(|(_, seq)| *seq).unwrap_or_default(),
                            ":limit": limit as i64,
                        },
                        |row| {
                            let mut item = item_from_row(row)?;
                            item.summary = table.read_row(row, 6)?;
                            Ok(item)
                        },
                    )?;
                    if statements.len() == 1 {
                        return Ok(rows);
                    }
                    // NOTE: a calendar's three tables each answer for
                    // every item; the row whose summary is there wins.
                    for row in rows.drain(..) {
                        match page.iter_mut().find(|held| held.seq == row.seq) {
                            Some(held) if held.summary.is_none() => held.summary = row.summary,
                            Some(_) => {}
                            None => page.push(row),
                        }
                    }
                }
                page.sort_by(|a, b| {
                    (a.sort_key.as_str(), a.seq).cmp(&(b.sort_key.as_str(), b.seq))
                });
                page.truncate(limit);
                Ok(page)
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
        )?;

        self.attach_addresses(collection, &tables, &mut items)?;
        Ok(items)
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
                    item_from_row,
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

    /// One live item by its public id with its summary and addresses, or
    /// `None`; a tombstone reads as `None`.
    pub fn get_item(&self, collection: &str, seq: i64) -> Result<Option<PimdirItem>, PimdirError> {
        let item = self.committed_item(collection, seq)?;
        if !self.overlay {
            return Ok(item);
        }

        let pending = self.pending(collection)?;
        let item = match item {
            Some(item) => Some(item),
            None => match pending.arrivals.get(&seq) {
                Some(from) => self.committed_item(from, seq)?,
                None => None,
            },
        };
        Ok(item.and_then(|item| fold(item, pending.edits.get(&seq))))
    }

    /// Resolves an item's public id from its key, for a consumer that
    /// just staged an add.
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

    /// Every source's binding of one item, keyed by source (§13).
    pub fn item_bindings(
        &self,
        collection: &str,
        link_id: &str,
    ) -> Result<BTreeMap<PimdirSourceId, PimdirBinding>, PimdirError> {
        Ok(rows(
            &self.conn,
            sql::LIST_ITEM_BINDINGS,
            named_params! { ":collection": collection, ":link_id": link_id },
            binding_from_row,
        )?
        .into_iter()
        .map(|(_, source, binding)| (source, binding))
        .collect())
    }

    /// The bindings waiting for a decision across one account's
    /// collections (§13); `None` lists an ungrouped store whole.
    pub fn list_conflicts(
        &self,
        account: Option<&str>,
    ) -> Result<Vec<PimdirConflict>, PimdirError> {
        Ok(rows(
            &self.conn,
            sql::LIST_CONFLICTED_BINDINGS,
            named_params! { ":account": account },
            |r| {
                Ok(PimdirConflict {
                    collection: r.get(0)?,
                    link_id: PimdirLinkId(r.get(1)?),
                    source: PimdirSourceId(r.get(2)?),
                    handle: PimdirHandle(r.get(3)?),
                    conflict_revision: r.get(4)?,
                    base_object: r.get::<_, Option<String>>(5)?.map(PimdirHash),
                    object: r.get::<_, Option<String>>(6)?.map(PimdirHash),
                    conflict_object: r.get::<_, Option<String>>(7)?.map(PimdirHash),
                })
            },
        )?)
    }

    /// A collection's live item count.
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

    /// How many of a collection's handles no read can list yet.
    pub fn count_probes(&self, collection: &str) -> Result<u64, PimdirError> {
        let count: i64 = self.conn.query_row(
            sql::COUNT_PROBES,
            named_params! { ":collection": collection },
            |r| r.get(0),
        )?;
        Ok(count.max(0) as u64)
    }

    /// Every live placement of one key, with its collection and account (§9.2).
    pub fn link_placements(&self, link_id: &str) -> Result<Vec<PimdirItemLocation>, PimdirError> {
        Ok(rows(
            &self.conn,
            sql::LIST_LINK_PLACEMENTS,
            named_params! { ":link_id": link_id },
            |r| {
                Ok(PimdirItemLocation {
                    collection: r.get(0)?,
                    account: r.get(1)?,
                    seq: r.get(2)?,
                    link_id: PimdirLinkId(link_id.to_string()),
                    object: r.get::<_, Option<String>>(3)?.map(PimdirHash),
                    flags: codec::flags_from_json(r.get::<_, Option<String>>(4)?.as_deref()),
                    level: codec::level_from_int(r.get(5)?),
                })
            },
        )?)
    }

    /// Every live placement of one body, the dedup axis (§9).
    pub fn object_placements(&self, hash: &str) -> Result<Vec<PimdirItemLocation>, PimdirError> {
        Ok(rows(
            &self.conn,
            sql::LIST_OBJECT_PLACEMENTS,
            named_params! { ":hash": hash },
            |r| {
                Ok(PimdirItemLocation {
                    collection: r.get(0)?,
                    account: r.get(1)?,
                    seq: r.get(2)?,
                    link_id: PimdirLinkId(r.get(3)?),
                    object: Some(PimdirHash(hash.to_string())),
                    flags: codec::flags_from_json(r.get::<_, Option<String>>(4)?.as_deref()),
                    level: codec::level_from_int(r.get(5)?),
                })
            },
        )?)
    }

    /// Every live placement naming one address, store-wide, `role` `None`
    /// for any role: the person axis (Annex A.6).
    pub fn address_placements(
        &self,
        address: &str,
        role: Option<PimdirAddressRole>,
    ) -> Result<Vec<PimdirAddressPlacement>, PimdirError> {
        Ok(rows(
            &self.conn,
            sql::LIST_ADDRESS_PLACEMENTS,
            named_params! { ":address": address, ":role": role.map(|r| r.as_str()) },
            |r| {
                Ok(PimdirAddressPlacement {
                    address: address.to_string(),
                    role: PimdirAddressRole::parse(&r.get::<_, String>(0)?)
                        .unwrap_or(PimdirAddressRole::From),
                    collection: r.get(1)?,
                    account: r.get(2)?,
                    kind: r.get(3)?,
                    seq: r.get(4)?,
                    sort_key: r.get(5)?,
                })
            },
        )?)
    }

    /// The same for one domain, by a scan.
    pub fn domain_placements(
        &self,
        domain: &str,
        role: Option<PimdirAddressRole>,
    ) -> Result<Vec<PimdirAddressPlacement>, PimdirError> {
        Ok(rows(
            &self.conn,
            sql::LIST_DOMAIN_PLACEMENTS,
            named_params! { ":domain": domain, ":role": role.map(|r| r.as_str()) },
            |r| {
                Ok(PimdirAddressPlacement {
                    address: r.get(0)?,
                    role: PimdirAddressRole::parse(&r.get::<_, String>(1)?)
                        .unwrap_or(PimdirAddressRole::From),
                    collection: r.get(2)?,
                    account: r.get(3)?,
                    kind: r.get(4)?,
                    seq: r.get(5)?,
                    sort_key: r.get(6)?,
                })
            },
        )?)
    }
}

/// Retention, the queue and the change feed.
impl PimdirReader {
    /// A keyset page of retained items (§11), cursor on `seq`, the trash view.
    pub fn list_retained(
        &self,
        collection: &PimdirCollectionId,
        after: Option<i64>,
        limit: usize,
    ) -> Result<Vec<PimdirItem>, PimdirError> {
        let mut items = rows(
            &self.conn,
            sql::LIST_RETAINED_PAGE,
            named_params! {
                ":collection": collection.0,
                ":after": after.unwrap_or(0),
                ":limit": limit as i64,
            },
            item_from_row,
        )?;
        self.attach_summaries(&collection.0, &mut items)?;
        Ok(items)
    }

    /// A collection's retained item count.
    pub fn count_retained(&self, collection: &PimdirCollectionId) -> Result<i64, PimdirError> {
        Ok(self.conn.query_row(
            sql::COUNT_RETAINED,
            named_params! { ":collection": collection.0 },
            |r| r.get(0),
        )?)
    }

    /// The bytes retention holds store-wide, each body counted once: an
    /// upper bound on what a purge would reclaim.
    pub fn retained_bytes(&self) -> Result<u64, PimdirError> {
        let bytes: i64 = self.conn.query_row(sql::RETAINED_BYTES, [], |r| r.get(0))?;
        Ok(bytes.max(0) as u64)
    }

    /// The collections with pending queue work.
    pub fn queued_collections(&self) -> Result<Vec<String>, PimdirError> {
        Ok(rows(&self.conn, sql::LIST_QUEUED_COLLECTIONS, [], |r| {
            r.get(0)
        })?)
    }

    /// A collection's pending actions in append order (§15.4).
    pub fn pending_actions(
        &self,
        collection: &str,
    ) -> Result<Vec<PimdirPendingAction>, PimdirError> {
        pending_actions(&self.conn, collection)
    }

    /// Every parked action across the store, in append order.
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

    /// The queued creates targeting a collection, reported apart since a
    /// create has no public id until the owner applies it.
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

    /// How many creates a collection has queued.
    pub fn count_pending_creates(&self, collection: &str) -> Result<usize, PimdirError> {
        Ok(self.pending_creates(collection)?.len())
    }

    /// The change feed's cursor (§4.5), recorded beside what a consumer derives.
    pub fn change_cursor(&self) -> Result<PimdirChangeCursor, PimdirError> {
        Ok(self.conn.query_row(sql::LOAD_CHANGE_CURSOR, [], |r| {
            Ok(PimdirChangeCursor {
                next_change: r.get(0)?,
                purges: r.get(1)?,
            })
        })?)
    }

    /// Every item stamped above `since`, retained ones included, in stamp order.
    pub fn items_changed_since(
        &self,
        since: i64,
        limit: usize,
    ) -> Result<Vec<PimdirItemChange>, PimdirError> {
        Ok(rows(
            &self.conn,
            sql::LIST_ITEMS_CHANGED_SINCE,
            named_params! { ":since": since, ":limit": limit as i64 },
            |r| {
                Ok(PimdirItemChange {
                    collection: r.get(0)?,
                    link_id: PimdirLinkId(r.get(1)?),
                    seq: r.get(2)?,
                    changed: r.get(3)?,
                    deleted: r.get::<_, i64>(4)? != 0,
                    retained_at: r.get(5)?,
                })
            },
        )?)
    }

    /// Every collection stamped above `since`, a renamed one under its new id.
    pub fn collections_changed_since(
        &self,
        since: i64,
        limit: usize,
    ) -> Result<Vec<PimdirCollectionChange>, PimdirError> {
        Ok(rows(
            &self.conn,
            sql::LIST_COLLECTIONS_CHANGED_SINCE,
            named_params! { ":since": since, ":limit": limit as i64 },
            |r| {
                Ok(PimdirCollectionChange {
                    id: r.get(0)?,
                    account: r.get(1)?,
                    kind: r.get(2)?,
                    name: r.get(3)?,
                    changed: r.get(4)?,
                })
            },
        )?)
    }
}

/// What the pending queue changes about one collection (§15.4).
#[derive(Debug, Default)]
struct PimdirPending {
    /// Actions restating or removing an item the collection holds, by public id.
    edits: BTreeMap<i64, Vec<PimdirAction>>,
    /// Items another collection's pending move or copy brings in, mapped
    /// to the collection their row is still read from.
    arrivals: BTreeMap<i64, String>,
}

impl PimdirPending {
    /// How many rows the fold can drop from a page.
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
    /// One live item as the committed rows hold it, with its summary.
    fn committed_item(
        &self,
        collection: &str,
        seq: i64,
    ) -> Result<Option<PimdirItem>, PimdirError> {
        let kind = kind_of(&self.conn, collection)?;
        let tables = tables_of(&kind);
        let statement = |table: PimdirSummaryTable| match table {
            PimdirSummaryTable::Mail => sql::GET_MAIL,
            PimdirSummaryTable::Contact => sql::GET_CONTACT,
            PimdirSummaryTable::Event => sql::GET_EVENT,
            PimdirSummaryTable::Task => sql::GET_TASK,
            PimdirSummaryTable::Journal => sql::GET_JOURNAL,
        };

        let mut found: Option<PimdirItem> = None;
        for table in &tables {
            let item = self
                .conn
                .query_row(
                    statement(*table),
                    named_params! { ":collection": collection, ":seq": seq },
                    |row| {
                        let mut item = item_from_row(row)?;
                        item.summary = table.read_row(row, 6)?;
                        Ok(item)
                    },
                )
                .optional()?;
            match (item, &mut found) {
                (None, _) => return Ok(None),
                (Some(item), None) => found = Some(item),
                (Some(item), Some(held)) if held.summary.is_none() => held.summary = item.summary,
                _ => {}
            }
        }

        let mut items: Vec<PimdirItem> = found.into_iter().collect();
        self.attach_addresses(collection, &tables, &mut items)?;
        Ok(items.pop())
    }

    /// Joins the summary rows and their addresses onto items read without
    /// them, the trash view's, two queries per page.
    fn attach_summaries(
        &self,
        collection: &str,
        items: &mut [PimdirItem],
    ) -> Result<(), PimdirError> {
        if items.is_empty() {
            return Ok(());
        }
        let tables = tables_of(&kind_of(&self.conn, collection)?);
        let links: Vec<String> = items.iter().map(|item| item.link_id.0.clone()).collect();
        let scope = serde_json::to_string(&links)?;
        for table in &tables {
            for (link, summary) in load_summaries(&self.conn, *table, collection, Some(&scope))? {
                if let Some(item) = items.iter_mut().find(|item| item.link_id == link) {
                    item.summary = Some(summary);
                }
            }
        }
        self.attach_addresses(collection, &tables, items)
    }

    /// Joins the address rows onto a page's summaries, one query per page.
    fn attach_addresses(
        &self,
        collection: &str,
        tables: &[PimdirSummaryTable],
        items: &mut [PimdirItem],
    ) -> Result<(), PimdirError> {
        if tables.is_empty() || items.iter().all(|item| item.summary.is_none()) {
            return Ok(());
        }
        let links: Vec<String> = items.iter().map(|item| item.link_id.0.clone()).collect();
        let scope = serde_json::to_string(&links)?;
        for (link, role, address) in load_addresses(&self.conn, collection, Some(&scope))? {
            if let Some(summary) = items
                .iter_mut()
                .find(|item| item.link_id == link)
                .and_then(|item| item.summary.as_mut())
            {
                crate::client::write::attach_address(summary, role, address);
            }
        }
        Ok(())
    }

    /// Folds the store's pending queue into what it changes about one
    /// collection, walked in global append order.
    fn pending(&self, collection: &str) -> Result<PimdirPending, PimdirError> {
        let mut queued = Vec::new();
        for from in self.queued_collections()? {
            for action in pending_actions(&self.conn, &from)? {
                queued.push((from.clone(), action));
            }
        }
        queued.sort_by_key(|(_, action)| action.id);

        let mut pending = PimdirPending::default();
        for (from, action) in queued {
            let here = from == collection;
            match &action.action {
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

    /// The items pending moves and copies bring into the collection.
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

    /// Folds the overlay into one page, reading past the limit by the
    /// pending removals so a page comes back short only where the
    /// collection ends.
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
}

/// Folds an item's pending actions into it, `None` when they take it out
/// of the collection: `set-flags` is absolute, `update` repoints the body.
fn fold(mut item: PimdirItem, actions: Option<&Vec<PimdirAction>>) -> Option<PimdirItem> {
    for action in actions.into_iter().flatten() {
        match action {
            PimdirAction::SetFlags { flags, .. } => item.flags = flags.clone(),
            PimdirAction::Update { object, .. } => {
                item.object = Some(object.clone());
                item.level = PimdirLevel::Full;
            }
            PimdirAction::Remove { .. } => return None,
            _ => {}
        }
    }
    Some(item)
}

/// Maps a `list_collections`-shaped row.
fn collection_from_row(r: &Row<'_>) -> rusqlite::Result<PimdirCollection> {
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

/// Maps the six item columns every item read leads with; the retained
/// page carries three more, which one mapper reads too.
pub(crate) fn item_from_row(row: &Row) -> rusqlite::Result<PimdirItem> {
    let seq: i64 = row.get(0)?;
    let link: String = row.get(1)?;
    let flags: Option<String> = row.get(2)?;
    let object: Option<String> = row.get(3)?;
    let sort_key: String = row.get(4)?;
    let level: i64 = row.get(5)?;

    let retention = match row.as_ref().column_name(6) {
        Ok("retained_at") => Some(PimdirRetention {
            at: row.get(6)?,
            by: row.get(7)?,
            size: row.get::<_, Option<i64>>(8)?.map(|size| size.max(0) as u64),
        }),
        _ => None,
    };

    Ok(PimdirItem {
        seq,
        link_id: PimdirLinkId(link),
        flags: codec::flags_from_json(flags.as_deref()),
        sort_key,
        object: object.map(PimdirHash),
        level: codec::level_from_int(level),
        summary: None,
        retention,
    })
}
