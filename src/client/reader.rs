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
    vec,
    vec::Vec,
};

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use rusqlite::{Connection, OpenFlags, OptionalExtension, Row, named_params};

use crate::{
    capability::PimdirSourceCapabilities,
    client::{
        PimdirError,
        blobs::PimdirBlobs,
        capability,
        producer::{
            PimdirActionStatus, PimdirParkedAction, PimdirPendingAction, action_status,
            overlaid_actions, pending_actions,
        },
        rows, schema,
        write::{
            PimdirSummaryTable, attach_address, binding_from_row, kind_of, load_addresses,
            load_summaries, tables_of,
        },
    },
    codec::{self, PimdirAction},
    collection::{PimdirCoverage, PimdirScope},
    hash::{PimdirHashAlgo, PimdirHasher},
    hub::{PimdirBinding, PimdirSourceId},
    object::PimdirHash,
    placement::{PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId},
    reference::{PimdirEndpoint, PimdirReference, PimdirReferenceOrigin, PimdirReferenceRole},
    sql,
    summary::{
        PimdirAddressRole, PimdirSummary,
        file::{self, PimdirFileSummary},
    },
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
    /// What the source states the collection is for (§14): a mail role
    /// (`inbox`, `sent`, `drafts`, `trash`, `junk`, `archive`, `all`,
    /// `flagged`, `important`), `default` for a calendar or an address
    /// book, `None` when the source states nothing or the store predates
    /// the column.
    pub role: Option<String>,
    /// The coverage the collection is listed with (§14.1): the narrowest
    /// of its sources' (the latest floor, the earliest ceiling, the oldest
    /// closing), `None` while one of them has never closed a round. A
    /// reader says "mail since" from it, and a search over the collection
    /// knows it is not exhaustive below it.
    pub coverage: Option<PimdirCoverage>,
}

/// One live item as a read reports it (STORAGE §14.1).
///
/// Its summary is joined on the reads that carry one.
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

/// What the trash view holds about a deleted item (§11): retained, or a
/// tombstone a source still binds and cannot remove yet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirRetention {
    /// The RFC 3339 instant the last binding vanished, `None` while a
    /// source still binds the row.
    pub at: Option<String>,
    /// The source whose removal retired the item; diagnostic.
    pub by: Option<String>,
    /// The body's size in bytes, `None` alongside an absent body.
    pub size: Option<u64>,
}

/// Where one identity or one body sits (STORAGE §9.2).
///
/// One live placement with the collection and account it occurs in. A
/// fact, not a verdict.
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

/// One item two sources disagree on (STORAGE §10, §14.1): the shared
/// body kept and the diverging one recorded, for a resolver to read both.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirItemConflict {
    /// The collection the item sits in.
    pub collection: String,
    /// The item's key.
    pub link_id: PimdirLinkId,
    /// The item's public id.
    pub seq: i64,
    /// The shared body, kept.
    pub object: Option<PimdirHash>,
    /// The diverging body the policy recorded.
    pub conflict_object: Option<PimdirHash>,
}

/// One binding waiting for a decision (STORAGE §14.1).
///
/// Carries the three bodies a resolver merges: the base, the item's
/// own, and the remote's.
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

/// One source's coverage of a collection and the round it has under way
/// (§14.1).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirSourceCoverage {
    /// The source.
    pub source: String,
    /// The scope of its last closed round and when it closed, `None`
    /// before one closed.
    pub coverage: Option<PimdirCoverage>,
    /// The round it has under way, if any.
    pub round: Option<PimdirRoundState>,
}

/// A round under way as a reader sees it: its scope and when it opened.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirRoundState {
    /// The scope the round lists.
    pub scope: PimdirScope,
    /// When it opened, an RFC 3339 instant.
    pub started_at: String,
}

/// The read and attachment chips of the mail reads (§14.1), `None` for
/// either way.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PimdirMailFilter {
    /// `Some(true)` read only, `Some(false)` unread only: a flag set
    /// holding no `\Seen`, an unknown one included, is unread.
    pub seen: Option<bool>,
    /// `Some(true)` with an attachment mark only, `Some(false)` without;
    /// a mark never examined matches neither.
    pub attachment: Option<bool>,
}

/// Where a page spanning collections resumes (mail, or a contact, calendar
/// or file search): the last entry of the page before.
///
/// One public id is shared by an identity's placements (§9.1), so the
/// collection breaks the tie.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirMailCursor {
    /// The entry's sort key.
    pub sort_key: String,
    /// Its public id.
    pub seq: i64,
    /// Its collection.
    pub collection: String,
}

/// One entry of a page spanning collections: mail, or a contact or file
/// search.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirMailEntry {
    /// The collection the placement sits in.
    pub collection: String,
    /// The item, with its summary and addresses.
    pub item: PimdirItem,
}

impl PimdirMailEntry {
    /// The cursor resuming the page after this entry.
    pub fn cursor(&self) -> PimdirMailCursor {
        PimdirMailCursor {
            sort_key: self.item.sort_key.clone(),
            seq: self.item.seq,
            collection: self.collection.clone(),
        }
    }
}

/// How much mail one day of the `Date` holds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirDayCount {
    /// The day as `YYYY-MM-DD`, `None` for the undated.
    pub day: Option<String>,
    /// The messages that day.
    pub count: u64,
}

/// What a range of mail weighs (`sum_mail`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PimdirMailSum {
    /// The messages in the range.
    pub count: u64,
    /// The summed size in octets of those whose size is known.
    pub size: u64,
    /// The messages whose size is unknown, left out of `size`.
    pub unknown: u64,
}

/// One file a message attaches (`list_attachments`, §14.3).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirAttachment {
    /// The file's key, `part:` for one standing for a part.
    pub link_id: PimdirLinkId,
    /// The collection of the placement read, the stand-in's when held.
    pub collection: String,
    /// The file's public id.
    pub seq: i64,
    /// Its summary there: name, media type, size and part.
    pub summary: Option<PimdirFileSummary>,
    /// A body a placement of the file holds, a saved copy's.
    pub object: Option<PimdirHash>,
}

/// One calendar item a search answers (`search_calendar`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirCalendarHit {
    /// The placement, its item carrying no summary.
    pub entry: PimdirMailEntry,
    /// Its component: `event`, `task` or `journal`.
    pub component: String,
    /// Its summary.
    pub title: String,
}

/// A reference's end as a link shows it (`describe_endpoint`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirEndpointView {
    /// The collection of its first live placement.
    pub collection: String,
    /// Its public id.
    pub seq: i64,
    /// A subject, a name, a summary or a file's name.
    pub title: Option<String>,
}

/// One stand-in of an account with its message (`list_attachments_by_account`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirAccountAttachment {
    /// The attachments collection holding the stand-in.
    pub collection: String,
    /// The stand-in's public id.
    pub seq: i64,
    /// Its `part:` key.
    pub link_id: PimdirLinkId,
    /// Its name, media type, size and part.
    pub summary: Option<PimdirFileSummary>,
    /// The message attaching it.
    pub message: PimdirLinkId,
    /// The collection of the message's placement read.
    pub message_collection: String,
    /// The message's public id.
    pub message_seq: i64,
    /// The message's sort key, its `Date`.
    pub message_sort_key: String,
    /// The message's sender, canonical.
    pub sender: Option<String>,
    /// The sender's display name.
    pub sender_name: Option<String>,
    /// The message's date, RFC 3339.
    pub date: Option<String>,
}

impl PimdirAccountAttachment {
    /// The cursor resuming the page after this entry.
    pub fn cursor(&self) -> PimdirAccountAttachmentCursor {
        PimdirAccountAttachmentCursor {
            message_sort_key: self.message_sort_key.clone(),
            seq: self.seq,
        }
    }
}

/// Where a page of an account's attachments resumes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirAccountAttachmentCursor {
    /// The message sort key of the last entry.
    pub message_sort_key: String,
    /// The stand-in's public id.
    pub seq: i64,
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

/// The change feed's cursor (§4.5): the last stamp drawn and the purge
/// count, recorded beside what a consumer derives.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PimdirChangeCursor {
    /// The last stamp drawn: the next look asks for every stamp above it.
    pub changed: i64,
    /// How many rows left without a stamp: purged items and collected objects.
    pub purges: i64,
}

impl PimdirReader {
    /// Opens an existing store rooted at `dir` to read, refusing one no
    /// owner has created ([`PimdirError::Uncreated`]) and one at another
    /// version. Takes no lock.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, PimdirError> {
        let dir = dir.as_ref();
        if !dir.join("pimdir.db").is_file() {
            return Err(PimdirError::Uncreated);
        }
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
        collection: impl AsRef<str>,
    ) -> Result<Option<Option<String>>, PimdirError> {
        let collection = collection.as_ref();
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
    pub fn collection_kind(
        &self,
        collection: impl AsRef<str>,
    ) -> Result<Option<String>, PimdirError> {
        let collection = collection.as_ref();
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
            &self.collections_sql(sql::LIST_COLLECTIONS)?,
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
            &self.collections_sql(sql::LIST_COLLECTIONS_BY_ACCOUNT)?,
            named_params! { ":account": account },
            collection_from_row,
        )?)
    }

    /// A collection listing as the store can answer it: one whose owner has
    /// not reconciled `collections.role` or the coverage columns yet reads
    /// them as `NULL` (§6).
    fn collections_sql(&self, statement: &'static str) -> Result<String, PimdirError> {
        let mut statement = String::from(statement);
        if !schema::has_column(&self.conn, "collections", "role")? {
            statement = statement.replace("c.generation, c.role", "c.generation, NULL AS role");
        }
        if !schema::has_column(&self.conn, "sources", "covered_at")?
            && let (Some(from), Some(to)) = (statement.find("LEFT JOIN ("), statement.find(") v"))
        {
            statement.replace_range(
                from..to + 3,
                "LEFT JOIN (SELECT NULL AS collection, NULL AS covered_since, \
                 NULL AS covered_until, NULL AS covered_at) v",
            );
        }
        Ok(statement)
    }

    /// The accounts owning at least one collection; not a configured roster.
    pub fn list_accounts(&self) -> Result<Vec<String>, PimdirError> {
        Ok(rows(&self.conn, sql::LIST_ACCOUNTS, [], |r| r.get(0))?)
    }

    /// A collection's handle-space epoch (§12), `None` for an unknown one.
    pub fn generation(&self, collection: impl AsRef<str>) -> Result<Option<i64>, PimdirError> {
        let collection = collection.as_ref();
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

    /// The sources binding one collection, which is what tells a sync
    /// whether it runs beside others (SYNC §5).
    pub fn collection_sources(
        &self,
        collection: impl AsRef<str>,
    ) -> Result<Vec<String>, PimdirError> {
        Ok(rows(
            &self.conn,
            sql::COLLECTION_SOURCES,
            named_params! { ":collection": collection.as_ref() },
            |r| r.get(0),
        )?)
    }
}

/// The item reads, live only and keyed by the public id.
impl PimdirReader {
    /// A keyset page of live items in link-id order, the sweep that sees
    /// every item once; `after` is the exclusive lower bound.
    pub fn list_items(
        &self,
        collection: impl AsRef<str>,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<PimdirItem>, PimdirError> {
        let collection = collection.as_ref();
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
        collection: impl AsRef<str>,
        after: Option<(&str, i64)>,
        limit: usize,
    ) -> Result<Vec<PimdirItem>, PimdirError> {
        let collection = collection.as_ref();
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
        collection: impl AsRef<str>,
        after: Option<(&str, i64)>,
        limit: usize,
    ) -> Result<Vec<PimdirItem>, PimdirError> {
        let collection = collection.as_ref();
        self.sorted_page(sql::LIST_ITEMS_PAGE_DESC, collection, after, limit, true)
    }

    /// A page in the kind's natural direction with each item's summary
    /// and addresses joined (§14.1): newest first for mail, ascending
    /// for contacts and calendars, a calendar's three tables merged.
    ///
    /// A collection whose kind was never declared pages without summaries.
    pub fn list_summaries(
        &self,
        collection: impl AsRef<str>,
        after: Option<(&str, i64)>,
        limit: usize,
    ) -> Result<Vec<PimdirItem>, PimdirError> {
        let collection = collection.as_ref();
        let kind = kind_of(&self.conn, collection)?;
        let tables = tables_of(&kind);
        let descending = kind.starts_with("message/rfc822");
        let statements: Vec<(&str, PimdirSummaryTable)> =
            match kind.split(';').next().unwrap_or_default().trim() {
                "message/rfc822" => vec![(sql::LIST_MAIL_PAGE_DESC, PimdirSummaryTable::Mail)],
                "text/vcard" => vec![(sql::LIST_CONTACTS_PAGE_ASC, PimdirSummaryTable::Contact)],
                file::KIND => vec![(sql::LIST_FILES_PAGE_ASC, PimdirSummaryTable::File)],
                "text/calendar" => vec![
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
    pub fn get_item(
        &self,
        collection: impl AsRef<str>,
        seq: i64,
    ) -> Result<Option<PimdirItem>, PimdirError> {
        let collection = collection.as_ref();
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
        collection: impl AsRef<str>,
        link_id: &str,
    ) -> Result<Option<i64>, PimdirError> {
        let collection = collection.as_ref();
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
        collection: impl AsRef<str>,
        link_id: &str,
    ) -> Result<BTreeMap<PimdirSourceId, PimdirBinding>, PimdirError> {
        let collection = collection.as_ref();
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

    /// The items two sources disagree on across one account's collections
    /// (§10); `None` lists an ungrouped store whole.
    pub fn list_item_conflicts(
        &self,
        account: Option<&str>,
    ) -> Result<Vec<PimdirItemConflict>, PimdirError> {
        Ok(rows(
            &self.conn,
            sql::LIST_CONFLICTED_ITEMS,
            named_params! { ":account": account },
            |r| {
                Ok(PimdirItemConflict {
                    collection: r.get(0)?,
                    link_id: PimdirLinkId(r.get(1)?),
                    seq: r.get(2)?,
                    object: r.get::<_, Option<String>>(3)?.map(PimdirHash),
                    conflict_object: r.get::<_, Option<String>>(4)?.map(PimdirHash),
                })
            },
        )?)
    }

    /// A collection's live item count.
    pub fn count_items(&self, collection: impl AsRef<str>) -> Result<u64, PimdirError> {
        let collection = collection.as_ref();
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
    /// for any role: the person axis (Annex A.6). A row under a role the
    /// format lacks is left out.
    pub fn address_placements(
        &self,
        address: &str,
        role: Option<PimdirAddressRole>,
    ) -> Result<Vec<PimdirAddressPlacement>, PimdirError> {
        let placements = rows(
            &self.conn,
            sql::LIST_ADDRESS_PLACEMENTS,
            named_params! { ":address": address, ":role": role.map(|r| r.as_str()) },
            |r| {
                let Some(role) = PimdirAddressRole::parse(&r.get::<_, String>(0)?) else {
                    return Ok(None);
                };
                Ok(Some(PimdirAddressPlacement {
                    address: address.to_string(),
                    role,
                    collection: r.get(1)?,
                    account: r.get(2)?,
                    kind: r.get(3)?,
                    seq: r.get(4)?,
                    sort_key: r.get(5)?,
                }))
            },
        )?;

        Ok(placements.into_iter().flatten().collect())
    }

    /// The same for one domain, by a scan.
    pub fn domain_placements(
        &self,
        domain: &str,
        role: Option<PimdirAddressRole>,
    ) -> Result<Vec<PimdirAddressPlacement>, PimdirError> {
        let placements = rows(
            &self.conn,
            sql::LIST_DOMAIN_PLACEMENTS,
            named_params! { ":domain": domain, ":role": role.map(|r| r.as_str()) },
            |r| {
                let Some(role) = PimdirAddressRole::parse(&r.get::<_, String>(1)?) else {
                    return Ok(None);
                };
                Ok(Some(PimdirAddressPlacement {
                    address: r.get(0)?,
                    role,
                    collection: r.get(2)?,
                    account: r.get(3)?,
                    kind: r.get(4)?,
                    seq: r.get(5)?,
                    sort_key: r.get(6)?,
                }))
            },
        )?;

        Ok(placements.into_iter().flatten().collect())
    }
}

/// Retention, the queue and the change feed.
impl PimdirReader {
    /// A keyset page of the trash (§11), cursor on `seq`: every deleted
    /// item, retained or still bound by a source that cannot remove it,
    /// which [`PimdirRetention::at`] tells apart.
    pub fn list_retained(
        &self,
        collection: impl AsRef<str>,
        after: Option<i64>,
        limit: usize,
    ) -> Result<Vec<PimdirItem>, PimdirError> {
        let collection = collection.as_ref();
        let mut items = rows(
            &self.conn,
            sql::LIST_RETAINED_PAGE,
            named_params! {
                ":collection": collection,
                ":after": after.unwrap_or(0),
                ":limit": limit as i64,
            },
            item_from_row,
        )?;
        self.attach_summaries(collection, &mut items)?;
        Ok(items)
    }

    /// A collection's trash count, every deleted row.
    pub fn count_retained(&self, collection: impl AsRef<str>) -> Result<i64, PimdirError> {
        Ok(self.conn.query_row(
            sql::COUNT_RETAINED,
            named_params! { ":collection": collection.as_ref() },
            |r| r.get(0),
        )?)
    }

    /// The bytes retention holds store-wide, each body counted once: an
    /// upper bound on what a purge would reclaim.
    pub fn retained_bytes(&self) -> Result<u64, PimdirError> {
        let bytes: i64 = self.conn.query_row(sql::RETAINED_BYTES, [], |r| r.get(0))?;
        Ok(bytes.max(0) as u64)
    }

    /// Every pending action store-wide in append order, the drain's order
    /// (§15.2).
    pub fn list_pending_actions(&self) -> Result<Vec<PimdirPendingAction>, PimdirError> {
        pending_actions(&self.conn, None)
    }

    /// Where the queue row `id` stands (§15.4): pending, parked, applied
    /// with the item an `add` created, or found nowhere.
    pub fn action_status(&self, id: i64) -> Result<PimdirActionStatus, PimdirError> {
        action_status(&self.conn, id)
    }

    /// A collection's pending actions in append order (§15.4).
    pub fn pending_actions(
        &self,
        collection: impl AsRef<str>,
    ) -> Result<Vec<PimdirPendingAction>, PimdirError> {
        let collection = collection.as_ref();
        pending_actions(&self.conn, Some(collection))
    }

    /// What every source syncing `collection` can do there (§15.6).
    pub fn capabilities(
        &self,
        collection: impl AsRef<str>,
    ) -> Result<Vec<PimdirSourceCapabilities>, PimdirError> {
        capability::at_collection(&self.conn, collection.as_ref())
    }

    /// What every source binding the item `seq` of `collection` can do
    /// there (§15.6).
    pub fn item_capabilities(
        &self,
        collection: impl AsRef<str>,
        seq: i64,
    ) -> Result<Vec<PimdirSourceCapabilities>, PimdirError> {
        capability::at_item(&self.conn, collection.as_ref(), seq)
    }

    /// The sources of `account` able to perform an intent capability, and
    /// the one the user chose among them, if any (§15.6).
    pub fn performers(
        &self,
        account: Option<&str>,
        capability: &str,
    ) -> Result<(Vec<String>, Option<String>), PimdirError> {
        Ok((
            capability::candidates(&self.conn, account, None, capability)?,
            capability::chosen(&self.conn, account, capability)?,
        ))
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
        collection: impl AsRef<str>,
    ) -> Result<Vec<PimdirPendingAction>, PimdirError> {
        let collection = collection.as_ref();
        Ok(self
            .pending_actions(collection)?
            .into_iter()
            .filter(|queued| matches!(queued.action, PimdirAction::Add { .. }))
            .collect())
    }

    /// How many creates a collection has queued.
    pub fn count_pending_creates(&self, collection: impl AsRef<str>) -> Result<usize, PimdirError> {
        let collection = collection.as_ref();
        Ok(self.pending_creates(collection)?.len())
    }

    /// The change feed's cursor (§4.5), recorded beside what a consumer derives.
    pub fn change_cursor(&self) -> Result<PimdirChangeCursor, PimdirError> {
        Ok(self.conn.query_row(sql::LOAD_CHANGE_CURSOR, [], |r| {
            Ok(PimdirChangeCursor {
                changed: r.get(0)?,
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

/// The mail reads (§14.1): counts and pages over a set of collections
/// under the read and attachment chips, and the sender and subject
/// search, so a list is sized by a count before it loads around the
/// scroll position. They read the committed rows, the pending queue not
/// folded in.
impl PimdirReader {
    /// One collection's coverage per source, with the round each has
    /// under way (`list_coverage`): what a reader says "mail since" from
    /// per account, or shows a first sync filling in by. A store its
    /// owner has not reconciled answers no coverage.
    pub fn list_coverage(
        &self,
        collection: impl AsRef<str>,
    ) -> Result<Vec<PimdirSourceCoverage>, PimdirError> {
        if !schema::has_column(&self.conn, "sources", "covered_at")? {
            return Ok(Vec::new());
        }
        Ok(rows(
            &self.conn,
            sql::LIST_COVERAGE,
            named_params! { ":collection": collection.as_ref() },
            |r| {
                let at: Option<String> = r.get(3)?;
                let started_at: Option<String> = r.get(4)?;
                Ok(PimdirSourceCoverage {
                    source: r.get(0)?,
                    coverage: match at {
                        Some(at) => Some(PimdirCoverage {
                            scope: PimdirScope {
                                since: r.get(1)?,
                                until: r.get(2)?,
                            },
                            at,
                        }),
                        None => None,
                    },
                    round: match started_at {
                        Some(started_at) => Some(PimdirRoundState {
                            scope: PimdirScope {
                                since: r.get(5)?,
                                until: r.get(6)?,
                            },
                            started_at,
                        }),
                        None => None,
                    },
                })
            },
        )?)
    }

    /// The references an item makes (`references_from`, §14.2), in the
    /// order of the other end and the role, whatever the state of either
    /// end; none on a store whose owner has not added the table yet.
    pub fn references_from(
        &self,
        endpoint: &PimdirEndpoint,
    ) -> Result<Vec<PimdirReference>, PimdirError> {
        if !schema::has_table(&self.conn, "item_reference")? {
            return Ok(Vec::new());
        }
        Ok(rows(
            &self.conn,
            sql::REFERENCES_FROM,
            named_params! { ":kind": endpoint.kind, ":link_id": endpoint.link_id.as_str() },
            reference_from_row,
        )?)
    }

    /// The files a message attaches (`list_attachments`, §14.3), read
    /// from the file collections of `account` (`None` in a single-account
    /// store), in the order recorded: each with its part, and the body a
    /// saved copy holds. None on a store whose owner has not added the
    /// references or the file summaries yet.
    pub fn list_attachments(
        &self,
        account: Option<&str>,
        message: &PimdirLinkId,
    ) -> Result<Vec<PimdirAttachment>, PimdirError> {
        if !schema::has_table(&self.conn, "item_reference")?
            || !schema::has_table(&self.conn, "file_summary")?
        {
            return Ok(Vec::new());
        }
        Ok(rows(
            &self.conn,
            sql::LIST_ATTACHMENTS,
            named_params! { ":account": account, ":link_id": message.as_str() },
            |row| {
                let summary = match PimdirSummaryTable::File.read_row(row, 3)? {
                    Some(PimdirSummary::File(file)) => Some(file),
                    _ => None,
                };
                Ok(PimdirAttachment {
                    link_id: PimdirLinkId(row.get(0)?),
                    collection: row.get(1)?,
                    seq: row.get(2)?,
                    summary,
                    object: row.get::<_, Option<String>>(7)?.map(PimdirHash),
                })
            },
        )?)
    }

    /// The references made to an item (`references_to`, §14.2), on
    /// [`references_from`](Self::references_from)'s terms.
    pub fn references_to(
        &self,
        endpoint: &PimdirEndpoint,
    ) -> Result<Vec<PimdirReference>, PimdirError> {
        if !schema::has_table(&self.conn, "item_reference")? {
            return Ok(Vec::new());
        }
        Ok(rows(
            &self.conn,
            sql::REFERENCES_TO,
            named_params! { ":kind": endpoint.kind, ":link_id": endpoint.link_id.as_str() },
            reference_from_row,
        )?)
    }

    /// How much live mail a set of collections holds under the chips
    /// (`count_mail`). `since` is a floor on the sort key, an RFC 3339
    /// instant: mail below it, and undated mail, is left out; `None` sets
    /// none.
    pub fn count_mail(
        &self,
        collections: &[impl AsRef<str>],
        filter: PimdirMailFilter,
        since: Option<&str>,
    ) -> Result<u64, PimdirError> {
        let count: i64 = self.conn.query_row(
            sql::COUNT_MAIL,
            named_params! {
                ":collections": collections_json(collections)?,
                ":seen": filter.seen,
                ":attachment": filter.attachment,
                ":since": since,
            },
            |r| r.get(0),
        )?;
        Ok(count.max(0) as u64)
    }

    /// [`count_mail`](Self::count_mail) per day of the `Date`
    /// (`count_mail_by_day`), newest day first and the undated last.
    /// `shift` is a SQLite date modifier moving the UTC instant to the
    /// reader's wall clock (`"+120 minutes"`), `None` reading UTC days.
    pub fn count_mail_by_day(
        &self,
        collections: &[impl AsRef<str>],
        filter: PimdirMailFilter,
        since: Option<&str>,
        shift: Option<&str>,
    ) -> Result<Vec<PimdirDayCount>, PimdirError> {
        Ok(rows(
            &self.conn,
            sql::COUNT_MAIL_BY_DAY,
            named_params! {
                ":collections": collections_json(collections)?,
                ":seen": filter.seen,
                ":attachment": filter.attachment,
                ":since": since,
                ":shift": shift,
            },
            |r| {
                Ok(PimdirDayCount {
                    day: r.get(0)?,
                    count: r.get::<_, i64>(1)?.max(0) as u64,
                })
            },
        )?)
    }

    /// The unread mail of each collection of a set under the attachment
    /// chip and above `since` as [`count_mail`](Self::count_mail) reads
    /// it (`count_unread`), a collection holding none left out.
    pub fn count_unread(
        &self,
        collections: &[impl AsRef<str>],
        attachment: Option<bool>,
        since: Option<&str>,
    ) -> Result<BTreeMap<String, u64>, PimdirError> {
        let counted = rows(
            &self.conn,
            sql::COUNT_UNREAD,
            named_params! {
                ":collections": collections_json(collections)?,
                ":attachment": attachment,
                ":since": since,
            },
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?.max(0) as u64)),
        )?;
        Ok(counted.into_iter().collect())
    }

    /// What [`count_mail`](Self::count_mail) counts with a sort key in
    /// `[since, until)`, either bound `None` for open (`sum_mail`): the
    /// messages, their known size and how many have none. Undated mail
    /// lies below every date, so only a range open below holds it.
    pub fn sum_mail(
        &self,
        collections: &[impl AsRef<str>],
        filter: PimdirMailFilter,
        held: Option<bool>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> Result<PimdirMailSum, PimdirError> {
        let (count, size, unknown): (i64, i64, i64) = self.conn.query_row(
            sql::SUM_MAIL,
            named_params! {
                ":collections": collections_json(collections)?,
                ":seen": filter.seen,
                ":attachment": filter.attachment,
                ":held": held,
                ":since": since,
                ":until": until,
            },
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        Ok(PimdirMailSum {
            count: count.max(0) as u64,
            size: size.max(0) as u64,
            unknown: unknown.max(0) as u64,
        })
    }

    /// One live mail row by key (`get_mail_row`), in
    /// [`list_mail_page_filtered`](Self::list_mail_page_filtered)'s shape,
    /// with its summary and addresses; `None` when no live row holds it.
    pub fn get_mail_row(
        &self,
        collection: impl AsRef<str>,
        link_id: &PimdirLinkId,
    ) -> Result<Option<PimdirMailEntry>, PimdirError> {
        let mut entries = rows(
            &self.conn,
            sql::GET_MAIL_ROW,
            named_params! { ":collection": collection.as_ref(), ":link_id": link_id.as_str() },
            |row| entry_from_row(row, Some(PimdirSummaryTable::Mail)),
        )?;
        self.attach_entry_addresses(&mut entries, PimdirSummaryTable::Mail)?;
        Ok(entries.pop())
    }

    /// Contacts of a set of collections whose name or an address matches
    /// a `LIKE` pattern (`search_contacts`), A to Z across the set, each
    /// with its summary and addresses; `after` resumes after an entry.
    pub fn search_contacts(
        &self,
        collections: &[impl AsRef<str>],
        pattern: &str,
        after: Option<&PimdirMailCursor>,
        limit: usize,
    ) -> Result<Vec<PimdirMailEntry>, PimdirError> {
        let mut entries = rows(
            &self.conn,
            sql::SEARCH_CONTACTS,
            named_params! {
                ":collections": collections_json(collections)?,
                ":pattern": pattern,
                ":after_key": after.map(|after| after.sort_key.as_str()),
                ":after_seq": after.map(|after| after.seq).unwrap_or_default(),
                ":after_collection": after.map(|after| after.collection.as_str()),
                ":limit": limit as i64,
            },
            |row| entry_from_row(row, Some(PimdirSummaryTable::Contact)),
        )?;
        self.attach_entry_addresses(&mut entries, PimdirSummaryTable::Contact)?;
        Ok(entries)
    }

    /// Files of a set of collections whose name matches a `LIKE` pattern
    /// (`search_files`), on [`search_contacts`](Self::search_contacts)'s
    /// terms.
    pub fn search_files(
        &self,
        collections: &[impl AsRef<str>],
        pattern: &str,
        after: Option<&PimdirMailCursor>,
        limit: usize,
    ) -> Result<Vec<PimdirMailEntry>, PimdirError> {
        if !schema::has_table(&self.conn, "file_summary")? {
            return Ok(Vec::new());
        }
        Ok(rows(
            &self.conn,
            sql::SEARCH_FILES,
            named_params! {
                ":collections": collections_json(collections)?,
                ":pattern": pattern,
                ":after_key": after.map(|after| after.sort_key.as_str()),
                ":after_seq": after.map(|after| after.seq).unwrap_or_default(),
                ":after_collection": after.map(|after| after.collection.as_str()),
                ":limit": limit as i64,
            },
            |row| entry_from_row(row, Some(PimdirSummaryTable::File)),
        )?)
    }

    /// Events, tasks and journals of a set of collections whose summary,
    /// or an event's location, matches a `LIKE` pattern
    /// (`search_calendar`), on [`search_contacts`](Self::search_contacts)'s
    /// terms, each with its component and title.
    pub fn search_calendar(
        &self,
        collections: &[impl AsRef<str>],
        pattern: &str,
        after: Option<&PimdirMailCursor>,
        limit: usize,
    ) -> Result<Vec<PimdirCalendarHit>, PimdirError> {
        Ok(rows(
            &self.conn,
            sql::SEARCH_CALENDAR,
            named_params! {
                ":collections": collections_json(collections)?,
                ":pattern": pattern,
                ":after_key": after.map(|after| after.sort_key.as_str()),
                ":after_seq": after.map(|after| after.seq).unwrap_or_default(),
                ":after_collection": after.map(|after| after.collection.as_str()),
                ":limit": limit as i64,
            },
            |row| {
                Ok(PimdirCalendarHit {
                    entry: entry_from_row(row, None)?,
                    component: row.get(7)?,
                    title: row.get(8)?,
                })
            },
        )?)
    }

    /// A reference's end as a link shows it (`describe_endpoint`, §14.2):
    /// its first live placement under the kind and a title from whichever
    /// summary it has; `None` when no live placement holds it.
    pub fn describe_endpoint(
        &self,
        endpoint: &PimdirEndpoint,
    ) -> Result<Option<PimdirEndpointView>, PimdirError> {
        if !schema::has_table(&self.conn, "file_summary")? {
            return Ok(None);
        }
        Ok(self
            .conn
            .query_row(
                sql::DESCRIBE_ENDPOINT,
                named_params! { ":kind": endpoint.kind, ":link_id": endpoint.link_id.as_str() },
                |row| {
                    Ok(PimdirEndpointView {
                        collection: row.get(0)?,
                        seq: row.get(1)?,
                        title: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    /// Every stand-in of an account's attachments collection with the
    /// message attaching it (`list_attachments_by_account`, §14.3), newest
    /// message first; `after` resumes after an entry. None on a store whose
    /// owner has not added the references or the file summaries yet.
    pub fn list_attachments_by_account(
        &self,
        account: Option<&str>,
        after: Option<&PimdirAccountAttachmentCursor>,
        limit: usize,
    ) -> Result<Vec<PimdirAccountAttachment>, PimdirError> {
        if !schema::has_table(&self.conn, "item_reference")?
            || !schema::has_table(&self.conn, "file_summary")?
        {
            return Ok(Vec::new());
        }
        Ok(rows(
            &self.conn,
            sql::LIST_ATTACHMENTS_BY_ACCOUNT,
            named_params! {
                ":account": account,
                ":after_key": after.map(|after| after.message_sort_key.as_str()),
                ":after_seq": after.map(|after| after.seq).unwrap_or_default(),
                ":limit": limit as i64,
            },
            |row| {
                let summary = match PimdirSummaryTable::File.read_row(row, 3)? {
                    Some(PimdirSummary::File(file)) => Some(file),
                    _ => None,
                };
                Ok(PimdirAccountAttachment {
                    collection: row.get(0)?,
                    seq: row.get(1)?,
                    link_id: PimdirLinkId(row.get(2)?),
                    summary,
                    message: PimdirLinkId(row.get(7)?),
                    message_collection: row.get(8)?,
                    message_seq: row.get(9)?,
                    message_sort_key: row.get(10)?,
                    sender: row.get(11)?,
                    sender_name: row.get(12)?,
                    date: row.get(13)?,
                })
            },
        )?)
    }

    /// A newest-first page of live mail over a set of collections under
    /// the chips and above `since` (`list_mail_page_filtered`), each with
    /// its summary and addresses; `after` is the last entry of the page
    /// before, `None` the first page.
    pub fn list_mail_page_filtered(
        &self,
        collections: &[impl AsRef<str>],
        filter: PimdirMailFilter,
        since: Option<&str>,
        after: Option<&PimdirMailCursor>,
        limit: usize,
    ) -> Result<Vec<PimdirMailEntry>, PimdirError> {
        let mut entries = rows(
            &self.conn,
            sql::LIST_MAIL_PAGE_FILTERED,
            named_params! {
                ":collections": collections_json(collections)?,
                ":seen": filter.seen,
                ":attachment": filter.attachment,
                ":since": since,
                ":after_key": after.map(|after| after.sort_key.as_str()),
                ":after_seq": after.map(|after| after.seq).unwrap_or_default(),
                ":after_collection": after.map(|after| after.collection.as_str()),
                ":limit": limit as i64,
            },
            |row| entry_from_row(row, Some(PimdirSummaryTable::Mail)),
        )?;
        self.attach_entry_addresses(&mut entries, PimdirSummaryTable::Mail)?;
        Ok(entries)
    }

    /// [`list_mail_page_filtered`](Self::list_mail_page_filtered) over
    /// the messages whose subject, sender or sender name matches the
    /// `LIKE` pattern (`search_mail`), [`like_pattern`] building one from
    /// the words searched, with no floor. Not the body, which SEARCH.md's
    /// index answers: a hit list says so.
    pub fn search_mail(
        &self,
        collections: &[impl AsRef<str>],
        pattern: &str,
        filter: PimdirMailFilter,
        after: Option<&PimdirMailCursor>,
        limit: usize,
    ) -> Result<Vec<PimdirMailEntry>, PimdirError> {
        let mut entries = rows(
            &self.conn,
            sql::SEARCH_MAIL,
            named_params! {
                ":collections": collections_json(collections)?,
                ":pattern": pattern,
                ":seen": filter.seen,
                ":attachment": filter.attachment,
                ":after_key": after.map(|after| after.sort_key.as_str()),
                ":after_seq": after.map(|after| after.seq).unwrap_or_default(),
                ":after_collection": after.map(|after| after.collection.as_str()),
                ":limit": limit as i64,
            },
            |row| entry_from_row(row, Some(PimdirSummaryTable::Mail)),
        )?;
        self.attach_entry_addresses(&mut entries, PimdirSummaryTable::Mail)?;
        Ok(entries)
    }

    /// Joins the address rows onto a page spanning collections, read for
    /// `table`, one query per collection the page holds.
    fn attach_entry_addresses(
        &self,
        entries: &mut [PimdirMailEntry],
        table: PimdirSummaryTable,
    ) -> Result<(), PimdirError> {
        let mut by_collection: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (at, entry) in entries.iter().enumerate() {
            by_collection
                .entry(entry.collection.clone())
                .or_default()
                .push(at);
        }
        for (collection, positions) in by_collection {
            let mut items: Vec<PimdirItem> = positions
                .iter()
                .map(|at| entries[*at].item.clone())
                .collect();
            self.attach_addresses(&collection, &[table], &mut items)?;
            for (at, item) in positions.into_iter().zip(items) {
                entries[at].item = item;
            }
        }
        Ok(())
    }
}

/// The `LIKE` pattern [`PimdirReader::search_mail`] matches: the words
/// searched as typed, `%` around them, and a literal `%`, `_` or `\`
/// escaped with `\`.
pub fn like_pattern(words: &str) -> String {
    let mut pattern = String::from("%");
    for char in words.trim().chars() {
        if matches!(char, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(char);
    }
    pattern.push('%');
    pattern
}

/// Maps an `item_reference` row in its canonical column order.
pub(crate) fn reference_from_row(row: &Row) -> rusqlite::Result<PimdirReference> {
    Ok(PimdirReference {
        from: PimdirEndpoint {
            kind: row.get(0)?,
            link_id: PimdirLinkId(row.get(1)?),
        },
        to: PimdirEndpoint {
            kind: row.get(2)?,
            link_id: PimdirLinkId(row.get(3)?),
        },
        role: PimdirReferenceRole::parse(&row.get::<_, String>(4)?),
        origin: PimdirReferenceOrigin::parse(&row.get::<_, String>(5)?),
        created_at: row.get(6)?,
    })
}

/// A set of collection ids as the JSON array the mail reads bind.
pub(crate) fn collections_json(collections: &[impl AsRef<str>]) -> Result<String, PimdirError> {
    let ids: Vec<&str> = collections.iter().map(AsRef::as_ref).collect();
    Ok(serde_json::to_string(&ids)?)
}

/// Maps a row spanning collections: the collection, the item columns,
/// then `table`'s summary columns when it names one.
fn entry_from_row(
    row: &Row,
    table: Option<PimdirSummaryTable>,
) -> rusqlite::Result<PimdirMailEntry> {
    let flags: Option<String> = row.get(3)?;
    let object: Option<String> = row.get(4)?;
    Ok(PimdirMailEntry {
        collection: row.get(0)?,
        item: PimdirItem {
            seq: row.get(1)?,
            link_id: PimdirLinkId(row.get(2)?),
            flags: codec::flags_from_json(flags.as_deref()),
            object: object.map(PimdirHash),
            sort_key: row.get(5)?,
            level: codec::level_from_int(row.get(6)?),
            summary: match table {
                Some(table) => table.read_row(row, 7)?,
                None => None,
            },
            retention: None,
        },
    })
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
            PimdirSummaryTable::File => sql::GET_FILE,
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
                attach_address(summary, role, address);
            }
        }
        Ok(())
    }

    /// Folds the store's pending queue into what it changes about one
    /// collection, walked in global append order; a row whose payload
    /// does not decode is skipped, the drain being what parks it.
    fn pending(&self, collection: &str) -> Result<PimdirPending, PimdirError> {
        let mut pending = PimdirPending::default();
        for action in overlaid_actions(&self.conn)? {
            let from = action.collection;
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
        role: r.get(9)?,
        coverage: match r.get::<_, Option<String>>(12)? {
            Some(at) => Some(PimdirCoverage {
                scope: PimdirScope {
                    since: r.get(10)?,
                    until: r.get(11)?,
                },
                at,
            }),
            None => None,
        },
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
