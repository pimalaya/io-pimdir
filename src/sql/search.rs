//! # Search SQL
//!
//! This crate's index schema and statements (spec/migrations/search/ and
//! spec/queries/search/, the index being the implementation's own under
//! SEARCH.md), generated at build time, each prepared on index.db with the
//! store attached read-only as `store`.
//!
//! The statements of this crate's own follow: the indexer's bookkeeping
//! around the canonical ones, none of which another implementation needs
//! to run the same index.

include!(concat!(env!("OUT_DIR"), "/search.rs"));

/// The FTS5 `tokenize` string in force (SEARCH §5), recorded in
/// `index_meta.tokenizer`: an index recording another is rebuilt. It folds
/// no diacritic, the text reaching it folded already.
pub const TOKENIZER: &str = "unicode61 remove_diacritics 0";

/// The version of SEARCH.md the index follows (SEARCH §2), which
/// `init_index_meta` writes in `index_meta.format_version`: this draft's
/// name. An index of the canonical shape at an earlier version, or naming
/// none, is rebuilt; one at a later version is refused.
pub const FORMAT_VERSION: &str = "search-draft-17";

/// The draft number of [`FORMAT_VERSION`], versions being ordered by it
/// below every release (SEARCH §2).
pub const FORMAT_DRAFT: u32 = 17;

/// The index shape this crate builds and reads (SEARCH §2), which
/// `init_index_meta` writes in `index_meta.shape`.
pub const SHAPE: &str = "canonical";

/// How many public ids the index records as split (`split`, SEARCH §3),
/// counted up to `:limit`: a text count evaluates that many hits per
/// placement (`split_seqs`) or reads each term's placements whole.
pub const SPLIT_HELD: &str = r#"SELECT count(*) FROM (SELECT 1 FROM split LIMIT :limit);"#;

/// The matches of a query's text terms (SEARCH §8.5), each read once
/// through `match_objects` or `match_summaries` before the query runs and
/// numbered by the term, so a query of many terms joins one table rather
/// than nesting a full-text query per term; and their sources
/// (`term_source`, [`CLEAR_TERM_SOURCE`]). Temporary, per connection,
/// written by a query client too.
pub const CREATE_TERM_MATCH: &str = r#"CREATE TEMP TABLE IF NOT EXISTS term_match (
    term       INTEGER NOT NULL,
    collection TEXT    NOT NULL,
    akey       TEXT    NOT NULL,
    seq        INTEGER NOT NULL,
    src        TEXT    NOT NULL,
    rank       REAL
);
CREATE INDEX IF NOT EXISTS temp.term_match_by_term ON term_match(term, akey, seq);
CREATE TEMP TABLE IF NOT EXISTS term_source (
    term INTEGER NOT NULL,
    src  INTEGER NOT NULL,
    id   INTEGER NOT NULL,
    PRIMARY KEY (term, src, id)
) WITHOUT ROWID;"#;

/// Empties `temp.term_match` before a query reads its terms' matches.
pub const CLEAR_TERM_MATCH: &str = r#"DELETE FROM temp.term_match;"#;

/// Empties `temp.term_source` before a query reads its terms' sources:
/// per text term, the bodies (`src` 0, `object.id`) and the summaries
/// (`src` 1, `placement.id`) its expression matches, read from the
/// full-text doclists alone, so a candidate is probed by a seek rather
/// than a full-text query.
pub const CLEAR_TERM_SOURCE: &str = r#"DELETE FROM temp.term_source;"#;

/// The store's highest public id, by a seek on `items_by_seq_global`: about
/// how many rows it holds, what a planner weighs a walk's rows by.
pub const ITEMS_HELD: &str = r#"SELECT ifnull(max(seq), 0) FROM store.items;"#;

/// How many accounts the store's collections belong to, none counting as
/// one.
pub const COUNT_ACCOUNTS: &str =
    r#"SELECT count(DISTINCT ifnull(account, '')) FROM store.collections;"#;

/// The account of the store's collections when they hold one, empty for
/// none; no row for a store holding no collection.
pub const ONE_ACCOUNT: &str = r#"SELECT ifnull(account, '') FROM store.collections LIMIT 1;"#;

/// How many bodies a query's terms match together (`Compiled::joint`),
/// counted up to `:limit`.
pub const JOINT_BODIES: &str = r#"SELECT count(*) FROM (SELECT 1 FROM object_text WHERE object_text MATCH :match LIMIT :limit);"#;

/// How many bodiless placements' summaries a query's terms match together,
/// counted up to `:limit`.
pub const JOINT_SUMMARIES: &str = r#"SELECT count(*) FROM (SELECT 1 FROM summary_text WHERE summary_text MATCH :match LIMIT :limit);"#;

/// The hits a page of text terms alone reads from its bitmaps, joined to
/// the store by one statement. Temporary, per connection.
pub const CREATE_HIT_PAGE: &str = r#"CREATE TEMP TABLE IF NOT EXISTS hit_page (
    akey TEXT    NOT NULL,
    seq  INTEGER NOT NULL
);"#;

/// Empties `temp.hit_page` before a page writes its hits.
pub const CLEAR_HIT_PAGE: &str = r#"DELETE FROM temp.hit_page;"#;

/// Writes one hit of a page into `temp.hit_page`.
pub const INSERT_HIT_PAGE: &str = r#"INSERT INTO temp.hit_page(akey, seq) VALUES (:akey, :seq);"#;

/// Every collection of the store with its account, empty for none.
pub const COLLECTION_ACCOUNTS: &str = r#"SELECT id, ifnull(account, '') FROM store.collections;"#;

/// Whether the index holds the store as it stands: no stamp drawn above
/// its cursor (`behind`, SEARCH §8.2) and no purge, collection removal or
/// rename it has not reconciled. Its placements are then the store's live
/// ones (SEARCH §3), so a hit read from them needs no seek into the store.
pub const INDEX_CURRENT: &str = r#"SELECT (SELECT next_change - 1 FROM store.store_meta WHERE id = 1) <= store_change
   AND (SELECT purges FROM store.store_meta WHERE id = 1) = store_purges
FROM index_meta WHERE id = 1;"#;

/// `match_object` answered from a term's sources in `temp.term_source`:
/// whether the body of hash `:hash` is among those text term `:term`
/// matches.
pub const SOURCE_OBJECT: &str = r#"SELECT 1 FROM temp.term_source
WHERE term = :term AND src = 0 AND id = (SELECT id FROM object WHERE hash = :hash);"#;

/// `match_summary` answered from a term's sources in `temp.term_source`:
/// whether the summary of placement `(:collection, :seq)` is among those
/// text term `:term` matches.
pub const SOURCE_SUMMARY: &str = r#"SELECT 1 FROM temp.term_source
WHERE term = :term AND src = 1
  AND id = (SELECT id FROM placement WHERE collection = :collection AND seq = :seq);"#;

/// The live items of the collections of the `collections` slot (STORAGE
/// §4.4) stamped above the index cursor: what an answer may see stale or
/// not at all. It seeks the change stamps past the cursor, never every
/// item of the collections (`items_by_seq`), and a client asks it only
/// while `behind` is above 0, no item being stamped above the cursor
/// otherwise.
pub const COUNT_UNINDEXED: &str = r#"SELECT count(*) FROM store.items INDEXED BY items_by_changed
WHERE changed > :since AND deleted = 0 AND retained_at IS NULL
  AND collection IN (SELECT value FROM temp.param WHERE slot = 'collections');"#;

/// The coverage of the sources of the collections of the `collections`
/// slot rolled up in one row (SEARCH §8.2): the highest floor and the
/// lowest ceiling of those that closed a round, and how many never closed
/// one. What `list_coverage` answers one collection at a time, read in
/// one statement for every collection of a query.
pub const ROLL_COVERAGE: &str = r#"SELECT max(covered_since) FILTER (WHERE covered_at IS NOT NULL),
       min(covered_until) FILTER (WHERE covered_at IS NOT NULL),
       count(*) - count(covered_at)
FROM store.sources
WHERE collection IN (SELECT value FROM temp.param WHERE slot = 'collections');"#;

/// Every collection's id, account and kind: what a query's conjuncts
/// narrow its coverage's collections by (SEARCH §8.2).
pub const SCOPE_COLLECTIONS: &str = r#"SELECT id, account, kind FROM store.collections;"#;

/// The id, account and kind of collection `?1`: a query naming one by a
/// conjunct reads its coverage's collections from it alone.
pub const SCOPE_COLLECTION: &str =
    r#"SELECT id, account, kind FROM store.collections WHERE id = ?1;"#;

/// Whether the store holds a calendar collection, what a query no conjunct
/// narrows asks before reading its date range's cut items.
pub const HOLDS_CALENDARS: &str =
    r#"SELECT EXISTS (SELECT 1 FROM store.collections WHERE kind = 'text/calendar');"#;

/// How many collections the store holds.
pub const COUNT_COLLECTIONS: &str = r#"SELECT count(*) FROM store.collections;"#;

/// The version of index.db as this connection sees it, moved by every
/// commit of another connection.
pub const INDEX_DATA_VERSION: &str = r#"PRAGMA main.data_version;"#;

/// The version of the attached store as this connection sees it, moved by
/// every commit of another connection.
pub const STORE_DATA_VERSION: &str = r#"PRAGMA store.data_version;"#;

/// The items an `id:` value names, every kind each is held under (SEARCH
/// §8): the item of `seq`, `NULL` for a value not of digits, and every
/// item of the link id, their live placements' kinds and keys, so
/// `linked` is asked once per pair.
pub const LOAD_ENDS: &str = r#"SELECT DISTINCT c.kind, i.link_id FROM store.items i
JOIN store.collections c ON c.id = i.collection
WHERE (i.seq = :seq OR i.link_id = :link_id)
  AND i.deleted = 0 AND i.retained_at IS NULL;"#;

/// One placement row as the index holds it: its id, the rowid its summary
/// text is keyed on, its link id and the hash of the body indexed for it,
/// `NULL` while bodiless or once the body's row is gone (SEARCH §3).
pub const LOAD_PLACEMENT: &str = r#"SELECT id, link_id, (SELECT o.hash FROM object o WHERE o.id = placement.object) FROM placement
WHERE collection = :collection AND seq = :seq;"#;

/// Every category a task states, each once: what `category:` folds in
/// Rust and compares with its folded value (SEARCH §8), so no SQL function
/// folds the stored side.
pub const TASK_CATEGORY_VALUES: &str =
    r#"SELECT DISTINCT value FROM store.summary_value WHERE role = 'category';"#;

/// The mail placements the index holds for one message id, `count` zero
/// for a message no placement holds: how many, the lowest sort key, the
/// lowest and highest one naming a date (SEARCH §9).
pub const MESSAGE_PRESENCE: &str = r#"SELECT count(*), min(i.sort_key), min(nullif(i.sort_key, '')), max(nullif(i.sort_key, ''))
FROM placement p
JOIN store.collections c ON c.id = p.collection
JOIN store.items i ON i.collection = p.collection AND i.seq = p.seq
WHERE p.link_id = :link_id AND c.kind = 'message/rfc822';"#;

/// A body the index holds for one message id, the source of its
/// `In-Reply-To` and `References` (SEARCH §9).
pub const MESSAGE_HASH: &str = r#"SELECT o.hash FROM placement p
JOIN object o ON o.id = p.object
JOIN store.collections c ON c.id = p.collection
WHERE p.link_id = :link_id AND c.kind = 'message/rfc822'
ORDER BY p.collection, p.seq LIMIT 1;"#;

/// The first `In-Reply-To` id of a message id's summary (STORAGE Annex
/// A.1), the parent of a message the index holds no body of (SEARCH §9).
pub const MESSAGE_IN_REPLY_TO: &str = r#"SELECT r.value FROM placement p
JOIN store.items i ON i.collection = p.collection AND i.link_id = p.link_id
JOIN store.summary_value r ON r.item = i.id AND r.role = 'in-reply-to'
WHERE p.link_id = :link_id
ORDER BY p.collection, p.seq, r.position LIMIT 1;"#;

/// One message row: its thread and its parent.
pub const LOAD_MESSAGE: &str = r#"SELECT thread, parent FROM message WHERE link_id = :link_id;"#;

/// The messages naming one as their parent.
pub const MESSAGE_CHILDREN: &str = r#"SELECT link_id FROM message WHERE parent = :link_id;"#;

/// The thread of one hit, `(account, seq)`, through any of its live mail
/// placements (SEARCH §9).
pub const HIT_THREAD: &str = r#"SELECT m.thread FROM store.items i
JOIN store.collections c ON c.id = i.collection
JOIN message m ON m.link_id = i.link_id
WHERE i.seq = :seq AND c.account IS :account AND c.kind = 'message/rfc822'
  AND i.deleted = 0 AND i.retained_at IS NULL
ORDER BY i.collection LIMIT 1;"#;

/// A thread as one account holds it (SEARCH §9): its live members there,
/// as `thread_members` names them, counted by `seq`, with their lowest and
/// highest sort key naming a date.
pub const THREAD_SPAN: &str = r#"SELECT count(DISTINCT i.seq), min(nullif(i.sort_key, '')), max(nullif(i.sort_key, ''))
FROM message m
JOIN placement p ON p.link_id = m.link_id
JOIN store.collections c ON c.id = p.collection
JOIN store.items i ON i.collection = p.collection AND i.seq = p.seq
WHERE m.thread = :thread AND c.account IS :account AND c.kind = 'message/rfc822'
  AND i.deleted = 0 AND i.retained_at IS NULL;"#;

/// The name of [`POSTING`]'s table.
pub const POSTING_TABLE: &str = "pimdir_fields";

/// The posting lists of the short fields, this crate's own table beside
/// the canonical schema, which the shape check leaves out as it does any
/// other table (SEARCH §2): `object_text`'s `title` and `from` columns
/// alone, under its tokenizer and keyed on its rowid, so a `subject:` or
/// `from:` word reads the bodies stating it there without decoding the
/// doclist of every body holding it anywhere. It keeps each match's
/// columns and no positions (`detail = column`), so it answers a word or
/// a prefix in one column alone, a phrase reading `object_text`.
pub const POSTING: &str = r#"CREATE VIRTUAL TABLE IF NOT EXISTS pimdir_fields USING fts5(
    title, "from",
    content = '', contentless_delete = 1, detail = column,
    tokenize = 'unicode61 remove_diacritics 0'
);"#;

/// A body's `title` and `from` text into [`POSTING`], beside its
/// `object_text` row.
pub const INSERT_POSTING: &str =
    r#"INSERT INTO pimdir_fields(rowid, title, "from") VALUES (:rowid, :title, :from);"#;

/// A body's row of [`POSTING`] dropped with its `object_text` row.
pub const DELETE_POSTING: &str = r#"DELETE FROM pimdir_fields WHERE rowid = :rowid;"#;

/// [`POSTING`] merged into one segment, beside `optimize_text`.
pub const OPTIMIZE_POSTING: &str =
    r#"INSERT INTO pimdir_fields(pimdir_fields) VALUES ('optimize');"#;

/// The bodies an expression of [`POSTING`] matches (`?1`, one column's
/// word or prefix), at most `?2`.
pub const MATCH_POSTING: &str =
    r#"SELECT rowid FROM pimdir_fields WHERE pimdir_fields MATCH ?1 LIMIT ?2;"#;

/// Every statement of this crate's own, paired with its constant name.
pub const OWN: &[(&str, &str)] = &[
    ("INSERT_POSTING", INSERT_POSTING),
    ("DELETE_POSTING", DELETE_POSTING),
    ("OPTIMIZE_POSTING", OPTIMIZE_POSTING),
    ("MATCH_POSTING", MATCH_POSTING),
    ("COUNT_UNINDEXED", COUNT_UNINDEXED),
    ("ROLL_COVERAGE", ROLL_COVERAGE),
    ("SCOPE_COLLECTIONS", SCOPE_COLLECTIONS),
    ("INDEX_DATA_VERSION", INDEX_DATA_VERSION),
    ("STORE_DATA_VERSION", STORE_DATA_VERSION),
    ("LOAD_ENDS", LOAD_ENDS),
    ("LOAD_PLACEMENT", LOAD_PLACEMENT),
    ("TASK_CATEGORY_VALUES", TASK_CATEGORY_VALUES),
    ("MESSAGE_PRESENCE", MESSAGE_PRESENCE),
    ("MESSAGE_HASH", MESSAGE_HASH),
    ("MESSAGE_IN_REPLY_TO", MESSAGE_IN_REPLY_TO),
    ("LOAD_MESSAGE", LOAD_MESSAGE),
    ("MESSAGE_CHILDREN", MESSAGE_CHILDREN),
    ("HIT_THREAD", HIT_THREAD),
    ("THREAD_SPAN", THREAD_SPAN),
    ("SPLIT_HELD", SPLIT_HELD),
];

/// Every search statement, canonical then this crate's own, paired with
/// its name.
pub fn all() -> impl Iterator<Item = (&'static str, &'static str)> {
    CANONICAL.iter().chain(OWN).copied()
}

#[cfg(test)]
mod tests {
    use alloc::{format, string::String, vec, vec::Vec};

    use rusqlite::{Connection, params_from_iter, types::Null};

    use crate::sql::{self, search::*};

    /// An index database with the store attached as `store`, both at the
    /// inlined schemas, and the connection's temporary tables.
    fn index() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("pimdir.db");
        Connection::open(&store)
            .unwrap()
            .execute_batch(sql::MIGRATION_0001)
            .unwrap();
        let conn = Connection::open(dir.path().join("index.db")).unwrap();
        conn.execute_batch(MIGRATION_0001).unwrap();
        conn.execute_batch(POSTING).unwrap();
        conn.execute("ATTACH DATABASE ?1 AS store", [store.to_str().unwrap()])
            .unwrap();
        conn.execute_batch(sql::CREATE_PARAM).unwrap();
        (dir, conn)
    }

    /// The steps of a statement's query plan, its parameters `NULL`.
    fn plan(conn: &Connection, statement: &str) -> Vec<String> {
        let mut plan = conn
            .prepare(&format!("EXPLAIN QUERY PLAN {statement}"))
            .unwrap();
        let nulls = vec![Null; plan.parameter_count()];
        plan.query_map(params_from_iter(nulls), |row| row.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// The unindexed count seeks the change stamps past the cursor, never
    /// walking each collection's items (`items_by_seq`, 41 ms at 3,311
    /// collections).
    #[test]
    fn the_unindexed_count_seeks_the_change_stamp() {
        let (_dir, conn) = index();
        let steps = plan(&conn, COUNT_UNINDEXED);
        let seek = "store.items USING INDEX items_by_changed (changed>?)";
        assert!(steps.iter().any(|step| step.contains(seek)), "{steps:?}");
        assert!(
            !steps.iter().any(|step| step.contains("items_by_seq")),
            "{steps:?}"
        );
    }

    /// The temporal axis reads the sources of a scope's collections by
    /// their key, in one statement.
    #[test]
    fn the_rolled_coverage_seeks_the_sources_by_collection() {
        let (_dir, conn) = index();
        let steps = plan(&conn, ROLL_COVERAGE);
        let seek = "store.sources USING INDEX sqlite_autoindex_sources_1 (collection=?)";
        assert!(steps.iter().any(|step| step.contains(seek)), "{steps:?}");
    }
}
