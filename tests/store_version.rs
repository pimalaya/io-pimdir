//! The store version contract (STORAGE §4.2, §6): a store a released
//! io-pimdir wrote opens as owner without losing a row; a store above this
//! crate's version, and a database that is not a pimdir store, are refused
//! before anything is written, by every role; a declared algorithm is
//! checked before anything is reconciled.
//!
//! tests/fixtures/store-v0.5.1, store-v0.7.0 and store-v0.8.0 were written by
//! those releases (tests/fixtures/store_gen.rs, run from a `git archive` of
//! the tags, and of 3b9154a for 0.8.0, which has none): four collections,
//! one of a media type outside today's closed set, six items with their
//! bindings, one retained, an applied, a parked and two pending queue rows,
//! one of them an `add` pinning its body; the 0.8.0 one an invitation
//! reference too.

#![cfg(feature = "client")]

use std::{fs, path::Path};

use io_pimdir::{
    client::{PimdirError, PimdirStore, producer::PimdirProducer, reader::PimdirReader},
    hash::PimdirHashAlgo,
};
use rusqlite::Connection;

/// A copy of a checked-in fixture store, never the fixture itself.
fn fixture(name: &str) -> tempfile::TempDir {
    let from = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&from, dir.path());
    dir
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn db(dir: &Path) -> Connection {
    Connection::open(dir.join("pimdir.db")).unwrap()
}

/// Every row the fixture stores hold, table by table.
const ROWS: [(&str, i64); 6] = [
    ("SELECT count(*) FROM collections", 4),
    ("SELECT count(*) FROM items", 6),
    (
        "SELECT count(*) FROM items WHERE retained_at IS NOT NULL",
        1,
    ),
    ("SELECT count(*) FROM bindings", 5),
    ("SELECT count(*) FROM objects", 7),
    ("SELECT count(*) FROM queue", 3),
];

fn released_store_opens_whole(name: &str, receipts: i64, references: i64) {
    let dir = fixture(name);

    // A reader and a producer read the unreconciled store as it stands
    // (§6): what it lacks reads as NULL or empty.
    let reader = PimdirReader::open(dir.path()).unwrap();
    assert_eq!(reader.list_collections().unwrap().len(), 4);
    assert_eq!(reader.list_items("INBOX", None, 10).unwrap().len(), 2);
    assert_eq!(reader.list_retained("INBOX", None, 10).unwrap().len(), 1);
    assert_eq!(reader.list_pending_actions().unwrap().len(), 2);
    assert_eq!(reader.parked_actions().unwrap().len(), 1);
    drop(reader);
    let producer = PimdirProducer::open(dir.path(), "test").unwrap();
    assert_eq!(producer.pending_actions("INBOX").unwrap().len(), 2);
    drop(producer);

    let store = PimdirStore::open(dir.path()).unwrap();
    let conn = db(dir.path());
    for (sql, rows) in ROWS {
        assert_eq!(count(&conn, sql), rows, "{name}: {sql}");
    }
    assert_eq!(count(&conn, "SELECT count(*) FROM receipts"), receipts);
    assert_eq!(
        count(&conn, "SELECT count(*) FROM item_reference"),
        references,
        "{name}: the references kept"
    );
    assert_eq!(
        count(&conn, "SELECT count(*) FROM queue WHERE error IS NOT NULL"),
        1,
        "{name}: the parked row is still parked"
    );
    assert_eq!(
        count(&conn, "SELECT count(*) FROM collections WHERE kind = ''"),
        1,
        "{name}: the custom kind recorded kindless (repair_kinds)"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM items WHERE level NOT IN (1, 2)"
        ),
        0
    );
    assert_eq!(
        count(
            &conn,
            "SELECT version * 100 + derivation FROM store_meta WHERE id = 1"
        ),
        102,
        "{name}: version 1, derivation 2"
    );
    assert_eq!(count(&conn, "PRAGMA user_version"), io_pimdir::sql::VERSION);
    let organised: (Option<i64>, Option<String>) = conn
        .query_row(
            "SELECT priority, (SELECT group_concat(value, ',') FROM (SELECT value \
             FROM task_category ORDER BY position)) FROM task_summary",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        organised,
        (Some(1), Some("work,home".into())),
        "{name}: the task organisation re-derived from the held body"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM mail_summary WHERE invitation IS NOT NULL"
        ),
        1,
        "{name}: the invitation re-derived from the held body"
    );
    assert!(store.refcount_drift().unwrap().is_empty(), "{name}");
    drop(conn);

    // The queue carries on: the pending add applies over its body.
    let mut store = store.for_source("imap");
    let report = store.drain().unwrap();
    assert_eq!(
        (report.applied, report.parked),
        (2, 0),
        "{name}: {report:?}"
    );
    assert_eq!(store.list_items("INBOX", None, 10).unwrap().len(), 3);

    // And the next open finds it current.
    drop(store);
    drop(PimdirStore::open_current(dir.path()).unwrap());
}

#[test]
fn a_store_released_by_0_5_1_opens_without_losing_a_row() {
    released_store_opens_whole("store-v0.5.1", 0, 0);
}

#[test]
fn a_store_released_by_0_7_0_opens_without_losing_a_row() {
    released_store_opens_whole("store-v0.7.0", 1, 0);
}

#[test]
fn a_store_released_by_0_8_0_opens_without_losing_a_row() {
    released_store_opens_whole("store-v0.8.0", 1, 1);
}

/// A store a later version wrote, here in rollback-journal mode, is refused
/// naming both versions and left byte for byte as it was (§6 step 1): the
/// owner's open never sets its journal mode, reconciles nor migrates it.
#[test]
fn a_store_above_this_version_is_refused_by_every_role_and_never_written() {
    let dir = fixture("store-v0.7.0");
    let conn = db(dir.path());
    conn.pragma_update(None, "user_version", io_pimdir::sql::VERSION + 1)
        .unwrap();
    conn.execute(
        "UPDATE store_meta SET version = ?1",
        [io_pimdir::sql::VERSION + 1],
    )
    .unwrap();
    conn.execute_batch("CREATE TABLE later (x INTEGER) STRICT")
        .unwrap();
    drop(conn);
    let before = fs::read(dir.path().join("pimdir.db")).unwrap();

    let refused = PimdirStore::open(dir.path()).err().unwrap();
    assert!(
        matches!(refused, PimdirError::Version { found } if found == io_pimdir::sql::VERSION + 1),
        "{refused:?}"
    );
    assert_eq!(refused.code(), "store-newer");
    assert!(matches!(
        PimdirReader::open(dir.path()),
        Err(PimdirError::Version { .. })
    ));
    assert!(matches!(
        PimdirProducer::open(dir.path(), "test"),
        Err(PimdirError::Version { .. })
    ));
    assert!(PimdirStore::open_current(dir.path()).is_err());
    assert_eq!(fs::read(dir.path().join("pimdir.db")).unwrap(), before);
}

/// A pimdir.db another program wrote is not a pimdir store (§3): refused,
/// nothing in it dropped.
#[test]
fn a_foreign_database_is_refused_and_left_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let conn = db(dir.path());
    conn.execute_batch("CREATE TABLE notes (body TEXT); INSERT INTO notes VALUES ('keep me');")
        .unwrap();
    drop(conn);
    let before = fs::read(dir.path().join("pimdir.db")).unwrap();
    let refused = PimdirStore::open(dir.path()).err().unwrap();
    assert!(matches!(refused, PimdirError::Foreign), "{refused:?}");
    assert_eq!(refused.code(), "store-foreign");
    assert_eq!(fs::read(dir.path().join("pimdir.db")).unwrap(), before);

    // One stamped and shaped like a store, but stating another format.
    let dir = fixture("store-v0.7.0");
    db(dir.path())
        .execute("UPDATE store_meta SET format = 'other'", [])
        .unwrap();
    let before = fs::read(dir.path().join("pimdir.db")).unwrap();
    assert!(matches!(
        PimdirStore::open(dir.path()),
        Err(PimdirError::Foreign)
    ));
    assert!(matches!(
        PimdirReader::open(dir.path()),
        Err(PimdirError::Foreign)
    ));
    assert_eq!(fs::read(dir.path().join("pimdir.db")).unwrap(), before);
}

/// A handle declaring another algorithm is refused before the store is
/// reconciled (§5, §6): nothing written under a name no reader finds.
#[test]
fn a_declared_algorithm_is_checked_before_anything_is_reconciled() {
    let dir = fixture("store-v0.7.0");
    let before = fs::read(dir.path().join("pimdir.db")).unwrap();
    let refused = PimdirStore::open_with_hash(dir.path(), Some(PimdirHashAlgo::Sha256_128)).err();
    assert!(
        matches!(refused, Some(PimdirError::HashAlgo { .. })),
        "{refused:?}"
    );
    assert_eq!(fs::read(dir.path().join("pimdir.db")).unwrap(), before);
}

/// A reader, a producer and an `open_current` owner tell a foreign
/// database from a store not created yet (§3): tables at `user_version` 0
/// are another program's, never `store-uncreated`.
#[test]
fn every_role_refuses_a_foreign_database_as_foreign() {
    let dir = tempfile::tempdir().unwrap();
    let conn = db(dir.path());
    conn.execute_batch("CREATE TABLE notes (body TEXT); INSERT INTO notes VALUES ('keep me');")
        .unwrap();
    drop(conn);
    let before = fs::read(dir.path().join("pimdir.db")).unwrap();

    let read = PimdirReader::open(dir.path()).err().unwrap();
    assert_eq!(read.code(), "store-foreign", "{read:?}");
    let produce = PimdirProducer::open(dir.path(), "app").err().unwrap();
    assert_eq!(produce.code(), "store-foreign", "{produce:?}");
    let own = PimdirStore::open_current(dir.path()).err().unwrap();
    assert_eq!(own.code(), "store-foreign", "{own:?}");
    assert_eq!(fs::read(dir.path().join("pimdir.db")).unwrap(), before);
}

/// A reader and a producer read a store an older release wrote as its
/// reconciliation would leave it before any backfill (STORAGE §6): what it
/// lacks reads as `NULL`, or as the column's canonical default, and a
/// missing table as empty, so a source holding no coverage columns never
/// closed a round, filling and never complete, and every read answers.
fn an_older_store_reads_as_its_reconciliation_would_leave_it(name: &str) {
    use io_pimdir::{
        client::reader::PimdirMailFilter, placement::PimdirLinkId, reference::PimdirEndpoint,
        search::coverage::PimdirSearchTemporal,
    };

    let dir = fixture(name);
    let conn = db(dir.path());
    conn.execute(
        "INSERT INTO sources(collection, source, checkpoint) VALUES('INBOX', 'imap', x'00')",
        [],
    )
    .unwrap();
    drop(conn);
    let before = fs::read(dir.path().join("pimdir.db")).unwrap();

    let reader = PimdirReader::open(dir.path()).unwrap();

    let all = PimdirMailFilter::default();
    let inbox = ["INBOX"];
    let message = PimdirLinkId::from("a@example.org");
    let card = PimdirEndpoint {
        kind: "text/vcard".into(),
        link_id: PimdirLinkId::from("urn:uuid:card-1"),
    };
    assert_eq!(reader.list_collections().unwrap().len(), 4, "{name}");
    reader.list_collections_by_account(None).unwrap();
    reader.list_accounts().unwrap();
    reader.distinct_sources().unwrap();
    reader.collection_sources("INBOX").unwrap();
    for collection in ["INBOX", "Contacts", "Tasks", "Lists"] {
        reader.list_items(collection, None, 10).unwrap();
        reader.list_items_page_asc(collection, None, 10).unwrap();
        reader.list_items_page_desc(collection, None, 10).unwrap();
        reader.list_summaries(collection, None, 10).unwrap();
        reader.capabilities(collection).unwrap();
        reader.accepts_vtodo(collection).unwrap();
        reader.pending_creates(collection).unwrap();
        reader.count_pending_creates(collection).unwrap();
        reader.count_retained(collection).unwrap();
    }
    for seq in 1..=6 {
        for collection in ["INBOX", "Contacts", "Tasks", "Lists"] {
            reader.get_item(collection, seq).unwrap();
            reader.item_capabilities(collection, seq).unwrap();
        }
    }
    reader.item_bindings("INBOX", "a@example.org").unwrap();
    reader.list_conflicts(None).unwrap();
    reader.list_item_conflicts(None).unwrap();
    reader.link_placements("a@example.org").unwrap();
    reader.address_placements("a@example.org", None).unwrap();
    reader.domain_placements("example.org", None).unwrap();
    reader.replied(&["a@example.org".into()]).unwrap();
    reader.retained_bytes().unwrap();
    reader.performers(None, "submit").unwrap();
    reader.change_cursor().unwrap();
    reader.items_changed_since(0, 10).unwrap();
    reader.collections_changed_since(0, 10).unwrap();
    reader.references_from(&card).unwrap();
    reader.references_to(&card).unwrap();
    reader.describe_endpoint(&card).unwrap();
    reader.list_attachments(None, &message).unwrap();
    reader.list_attachments_by_account(None, None, 10).unwrap();
    reader.count_mail(&inbox, all, None).unwrap();
    reader.count_mail_by_day(&inbox, all, None, None).unwrap();
    reader.count_unread(&inbox, None, None).unwrap();
    reader.sum_mail(&inbox, all, None, None, None).unwrap();
    reader.get_mail_row("INBOX", &message).unwrap();
    reader
        .search_contacts(&["Contacts"], "a", None, 10)
        .unwrap();
    reader.search_files(&inbox, "a", None, 10).unwrap();
    reader.search_calendar(&["Tasks"], "a", None, 10).unwrap();
    reader
        .list_mail_page_filtered(&inbox, all, None, None, 10)
        .unwrap();
    reader.search_mail(&inbox, "a", all, None, 10).unwrap();
    for action in reader.list_pending_actions().unwrap() {
        reader.action_status(action.id).unwrap();
    }
    let coverage = reader.list_coverage("INBOX").unwrap();
    assert_eq!(coverage.len(), 1, "{name}: the source is listed");
    assert_eq!(coverage[0].source, "imap");
    assert_eq!(coverage[0].coverage, None, "{name}: never closed");
    assert_eq!(coverage[0].round, None);
    assert!(
        matches!(
            reader
                .search_coverage(&["INBOX"], None, None)
                .unwrap()
                .temporal,
            PimdirSearchTemporal::Bounded { filling: true, .. }
        ),
        "{name}: filling, never complete"
    );
    drop(reader);

    let producer = PimdirProducer::open(dir.path(), "test").unwrap();
    producer.capabilities("INBOX").unwrap();
    if let Err(err) = producer.performer("INBOX", "submit", None) {
        assert_ne!(err.code(), "sql", "{name}: {err}");
    }
    for action in producer.pending_actions("INBOX").unwrap() {
        producer.action_status(action.id).unwrap();
    }
    drop(producer);

    assert_eq!(
        fs::read(dir.path().join("pimdir.db")).unwrap(),
        before,
        "{name}: nothing written"
    );
}

#[test]
fn a_store_released_by_0_5_1_reads_as_its_reconciliation_would_leave_it() {
    an_older_store_reads_as_its_reconciliation_would_leave_it("store-v0.5.1");
}

#[test]
fn a_store_released_by_0_7_0_reads_as_its_reconciliation_would_leave_it() {
    an_older_store_reads_as_its_reconciliation_would_leave_it("store-v0.7.0");
}

#[test]
fn a_store_released_by_0_8_0_reads_as_its_reconciliation_would_leave_it() {
    an_older_store_reads_as_its_reconciliation_would_leave_it("store-v0.8.0");
}

/// pimdir.db holds the canonical schema alone (STORAGE §6): another
/// implementation's table, column, index and trigger in a version 1 store
/// are dropped by its owner's reconciliation, every canonical row kept, and
/// the next `open_current` finds the store current.
#[test]
fn the_reconciliation_drops_what_the_canonical_schema_lacks() {
    let dir = tempfile::tempdir().unwrap();
    let store = PimdirStore::open(dir.path()).unwrap();
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    drop(store);

    let conn = db(dir.path());
    conn.execute_batch(
        "CREATE TABLE other_state (key TEXT PRIMARY KEY, value TEXT) STRICT;
         INSERT INTO other_state VALUES ('cursor', '42');
         ALTER TABLE collections ADD COLUMN other_color TEXT;
         CREATE INDEX other_by_name ON collections(name);
         CREATE TRIGGER other_on_insert AFTER INSERT ON collections
         BEGIN UPDATE other_state SET value = NEW.id; END;",
    )
    .unwrap();
    drop(conn);
    assert!(
        PimdirStore::open_current(dir.path()).is_err(),
        "not current"
    );

    drop(PimdirStore::open(dir.path()).unwrap());
    let conn = db(dir.path());
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM sqlite_schema \
             WHERE name IN ('other_state', 'other_by_name', 'other_on_insert')"
        ),
        0,
        "the table, index and trigger dropped"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM pragma_table_info('collections') WHERE name = 'other_color'"
        ),
        0,
        "the column dropped"
    );
    assert_eq!(count(&conn, "SELECT count(*) FROM collections"), 1);
    drop(conn);
    drop(PimdirStore::open_current(dir.path()).unwrap());
}

/// The capability and queue reads of a reader and a producer on a store an
/// older release wrote, as it stands, and on a copy its owner reconciled
/// (STORAGE §6): every reader of one older store answers alike, so what the
/// store lacks reads as the reconciliation leaves it, a missing
/// `capabilities` table as empty, its syncing sources then undeclared.
fn an_older_store_declares_as_its_reconciliation_would(name: &str) {
    let older = fixture(name);
    let reconciled = fixture(name);
    drop(PimdirStore::open(reconciled.path()).unwrap());

    type Answers = Vec<String>;
    let answers = |dir: &Path| -> Answers {
        let reader = PimdirReader::open(dir).unwrap();
        let producer = PimdirProducer::open(dir, "test").unwrap();
        let mut answers = Vec::new();
        for collection in ["INBOX", "Contacts", "Tasks", "Lists"] {
            answers.push(format!("{:?}", reader.capabilities(collection)));
            answers.push(format!("{:?}", reader.accepts_vtodo(collection)));
            answers.push(format!("{:?}", producer.capabilities(collection)));
            for capability in ["mail-submit", "collection-create"] {
                let performer = producer.performer(collection, capability, None);
                answers.push(format!("{:?}", performer.map_err(|err| err.code())));
            }
            for seq in 1..=6 {
                answers.push(format!("{:?}", reader.item_capabilities(collection, seq)));
            }
        }
        for capability in ["mail-submit", "calendar-reply"] {
            answers.push(format!("{:?}", reader.performers(None, capability)));
        }
        for id in 1..=6 {
            answers.push(format!("{:?}", producer.action_status(id)));
        }
        answers
    };

    let before = fs::read(older.path().join("pimdir.db")).unwrap();
    assert_eq!(answers(older.path()), answers(reconciled.path()), "{name}");
    assert_eq!(
        fs::read(older.path().join("pimdir.db")).unwrap(),
        before,
        "{name}: nothing written"
    );
}

#[test]
fn a_store_released_by_0_5_1_declares_as_its_reconciliation_would() {
    an_older_store_declares_as_its_reconciliation_would("store-v0.5.1");
}

#[test]
fn a_store_released_by_0_7_0_declares_as_its_reconciliation_would() {
    an_older_store_declares_as_its_reconciliation_would("store-v0.7.0");
}

#[test]
fn a_store_released_by_0_8_0_declares_as_its_reconciliation_would() {
    an_older_store_declares_as_its_reconciliation_would("store-v0.8.0");
}

/// The search index over a store an older release wrote, unreconciled
/// (STORAGE §6, SEARCH §3): the indexer is a store reader, so it builds
/// and every field of SEARCH §8 answers, what the store lacks read as the
/// reconciliation leaves it before any backfill.
#[cfg(feature = "search")]
fn an_older_store_is_indexed_and_searched(name: &str) {
    use io_pimdir::{
        client::search::{
            PimdirIndex, PimdirIndexRefreshOptions, PimdirSearchClock, PimdirSearchOptions,
            PimdirSearchSort,
        },
        search::query::PimdirQuery,
    };

    let dir = fixture(name);
    let before = fs::read(dir.path().join("pimdir.db")).unwrap();
    let mut index = PimdirIndex::open(dir.path()).unwrap();
    let report = index
        .refresh(&PimdirIndexRefreshOptions {
            now: Some(1_793_000_000),
            ..Default::default()
        })
        .unwrap();
    assert!(report.folded > 0, "{name}: {report:?}");

    let clock = PimdirSearchClock {
        now: Some(1_793_000_000),
    };
    let hits = |query: &str| -> Vec<i64> {
        let query = PimdirQuery::parse(query).unwrap();
        let mut seqs = Vec::new();
        for sort in [
            PimdirSearchSort::Date,
            PimdirSearchSort::Name,
            PimdirSearchSort::Relevance,
        ] {
            let options = PimdirSearchOptions {
                sort,
                clock,
                ..Default::default()
            };
            let found = index
                .search(&query, &options)
                .unwrap_or_else(|err| panic!("{name}: {query:?}: {err}"));
            index.threads(&query, &options).unwrap();
            let mut found: Vec<i64> = found.hits.iter().map(|hit| hit.seq).collect();
            found.sort();
            seqs = found;
        }
        assert_eq!(
            index.count(&query, &clock).unwrap().count,
            seqs.len() as u64
        );
        seqs
    };

    for query in [
        "first",
        "subject:first",
        "body:hello",
        "from:alice@example.org",
        "from:@example.org",
        "from:alice",
        "with:bob",
        "person:alice",
        "tag:x",
        "flag:x",
        "is:unread",
        "is:read",
        "is:encrypted",
        "has:body",
        "has:attachment",
        "kind:mail",
        "kind:contact",
        "kind:event",
        "kind:journal",
        "kind:file",
        "status:needs-action",
        "priority:high",
        "category:work",
        "account:x",
        "collection:INBOX",
        "date:2026",
        "thread:a@example.org",
        "id:1",
        "id:a@example.org",
        "linked:urn:uuid:card-1",
        "linked:invitation:meet-1@example.org",
        "attachment:pdf",
        "location:x",
        "changed:1..",
        "not is:unread",
        "first or not kind:mail",
    ] {
        hits(query);
    }
    // NOTE: the task's organisation columns are absent, so its priority is
    // none and its categories none (STORAGE §6), until its owner backfills.
    let tasks = hits("kind:task");
    assert_eq!(tasks.len(), 1, "{name}");
    assert_eq!(hits("priority:none"), tasks, "{name}");
    assert!(hits("category:work").is_empty(), "{name}");
    for seq in 1..=6 {
        index.item(None, seq).unwrap();
        index.thread(None, seq).unwrap();
    }
    drop(index);

    assert_eq!(
        fs::read(dir.path().join("pimdir.db")).unwrap(),
        before,
        "{name}: the store not written"
    );
}

#[cfg(feature = "search")]
#[test]
fn a_store_released_by_0_5_1_is_indexed_and_searched() {
    an_older_store_is_indexed_and_searched("store-v0.5.1");
}

#[cfg(feature = "search")]
#[test]
fn a_store_released_by_0_7_0_is_indexed_and_searched() {
    an_older_store_is_indexed_and_searched("store-v0.7.0");
}

#[cfg(feature = "search")]
#[test]
fn a_store_released_by_0_8_0_is_indexed_and_searched() {
    an_older_store_is_indexed_and_searched("store-v0.8.0");
}
