//! Reconciling an earlier draft's shape on a store that holds data
//! (spec §6, the `draft` allowance).
//!
//! The empty-store case in draft_reconcile.rs proves the `ALTER TABLE`
//! runs. What an upgrade actually meets is a populated store: items,
//! bindings, a conflict waiting for a person, a retained row, a queued
//! action. A column added there is a statement about rows written before
//! it existed, and §6 makes that normative, "a reconciled column MUST
//! also be backfilled wherever `NULL` is not the value the existing rows
//! already imply", with `bindings.shared_object` named as the case.
//!
//! A real old store is missing several columns at once and the code paths
//! interact, so the columns are dropped one at a time and then in random
//! subsets, from one populated store built the same way every time.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use io_pimdir::{PimdirSourceStore, PimdirStore, codec::PimdirAction, sql};
use io_replica::{
    change::{ReplicaDropReason, ReplicaWriteOp},
    client::ReplicaStorage,
    collection::ReplicaCollectionId,
    object::ReplicaObject,
    placement::{
        ReplicaBase, ReplicaFlags, ReplicaHandle, ReplicaLevel, ReplicaLinkId, ReplicaMeta,
        ReplicaPlacement, ReplicaSortKey, ReplicaStatus,
    },
};
use proptest::{
    prelude::*,
    test_runner::{Config, FileFailurePersistence},
};
use rusqlite::Connection;

/// The columns folded into version 1 after it was first published, as
/// `(table, column)`. Mirrors `reconcile_draft_shape`'s own list; a fold
/// added there and not here narrows this test silently, which is what
/// `every_folded_in_column_is_covered` refuses.
const FOLDED_IN: &[(&str, &str)] = &[
    ("bindings", "conflicted"),
    ("bindings", "conflict_revision"),
    ("bindings", "conflict_object"),
    ("bindings", "shared_object"),
    ("items", "retained_at"),
    ("items", "retained_by"),
    ("items", "sort_key"),
    ("collections", "account"),
    ("bindings", "base_present"),
];

/// The indexes over folded-in columns. SQLite refuses to drop a column an
/// index names, partial predicates included, so simulating an older store
/// takes these out first; the reconciliation is what puts them back.
const FOLDED_INDEXES: &[&str] = &[
    "items_retained",
    "collections_by_account",
    "items_by_sort",
    "items_by_seq_global",
    "objects_garbage",
    "items_by_conflict_object",
    "bindings_by_conflict_object",
    "bindings_conflicted",
    "queue_by_object",
    "bindings_by_handle",
];

const INBOX: &str = "INBOX";
const CARDS: &str = "Cards";

fn object(store: &PimdirSourceStore, bytes: &[u8]) -> ReplicaWriteOp {
    ReplicaWriteOp::StoreObject {
        object: ReplicaObject {
            hash: store.hash(bytes),
            size: bytes.len(),
        },
        body: Some(bytes.to_vec()),
    }
}

#[allow(clippy::too_many_arguments)]
fn placement(
    store: &PimdirSourceStore,
    collection: &str,
    handle: &str,
    link: &str,
    body: Option<&[u8]>,
    base: Option<&[u8]>,
    status: ReplicaStatus,
    conflict: Option<&[u8]>,
) -> ReplicaWriteOp {
    ReplicaWriteOp::UpsertPlacement(ReplicaPlacement {
        collection: ReplicaCollectionId(collection.into()),
        handle: ReplicaHandle(handle.into()),
        link_id: Some(ReplicaLinkId(link.into())),
        object: body.map(|bytes| store.hash(bytes)),
        level: ReplicaLevel::Full,
        meta: Some(ReplicaMeta(r#"{"v":1}"#.into())),
        sort_key: ReplicaSortKey(format!("k-{link}")),
        flags: ReplicaFlags::from_iter(["\\Seen"]),
        status,
        conflict_revision: (status == ReplicaStatus::Conflict).then(|| "r-remote".to_string()),
        conflict_object: conflict.map(|bytes| store.hash(bytes)),
        base: base.map(|bytes| ReplicaBase {
            flags: ReplicaFlags::default(),
            revision: Some("r-base".into()),
            object: Some(store.hash(bytes)),
        }),
        origin: None,
    })
}

/// Builds the store an upgrade meets: two collections under two accounts,
/// several sources, a conflict waiting for a person, a retained row and a
/// queued action pinning a body.
fn populate(dir: &Path) {
    let mut left = PimdirStore::open(dir).unwrap().for_source("left");
    left.ensure_collection(INBOX, "message/rfc822").unwrap();
    left.write(vec![
        object(&left, b"one"),
        object(&left, b"two"),
        object(&left, b"remote"),
        placement(
            &left,
            INBOX,
            "1",
            "mid:a",
            Some(b"one"),
            Some(b"one"),
            ReplicaStatus::Clean,
            None,
        ),
        placement(
            &left,
            INBOX,
            "2",
            "mid:b",
            Some(b"two"),
            Some(b"two"),
            ReplicaStatus::Conflict,
            Some(b"remote"),
        ),
        placement(
            &left,
            INBOX,
            "3",
            "mid:c",
            None,
            None,
            ReplicaStatus::Clean,
            None,
        ),
    ])
    .unwrap();

    let mut right = PimdirStore::open(dir).unwrap().for_source("right");
    right
        .write(vec![
            object(&right, b"one"),
            placement(
                &right,
                INBOX,
                "r1",
                "mid:a",
                Some(b"one"),
                None,
                ReplicaStatus::Clean,
                None,
            ),
        ])
        .unwrap();

    // The retained row: an item both sources drop, which retires rather
    // than deletes and keeps its body pinned (spec §11).
    for (store, handle) in [(&mut left, "3"), (&mut right, "r3")] {
        let _ = store.write(vec![ReplicaWriteOp::DropPlacement {
            collection: ReplicaCollectionId(INBOX.into()),
            handle: ReplicaHandle(handle.into()),
            reason: ReplicaDropReason::Deleted,
        }]);
    }
    let mut retiring = PimdirStore::open(dir).unwrap().for_source("left");
    retiring
        .write(vec![
            object(&retiring, b"gone"),
            placement(
                &retiring,
                INBOX,
                "9",
                "mid:gone",
                Some(b"gone"),
                Some(b"gone"),
                ReplicaStatus::Clean,
                None,
            ),
        ])
        .unwrap();
    retiring
        .write(vec![ReplicaWriteOp::DropPlacement {
            collection: ReplicaCollectionId(INBOX.into()),
            handle: ReplicaHandle("9".into()),
            reason: ReplicaDropReason::Deleted,
        }])
        .unwrap();

    // A second collection under an account, so the account column has
    // something to lose.
    let mut cards = PimdirStore::open(dir)
        .unwrap()
        .for_account("work")
        .for_source("left");
    cards.ensure_collection(CARDS, "text/vcard").unwrap();
    cards
        .write(vec![
            object(&cards, b"card"),
            placement(
                &cards,
                CARDS,
                "c1",
                "uid:x",
                Some(b"card"),
                Some(b"card"),
                ReplicaStatus::Clean,
                None,
            ),
        ])
        .unwrap();

    // A queued action pinning a body, so the fifth pointer column is
    // populated too.
    let queued = left.hash(b"queued");
    let blobs = left.blobs();
    let mut writer = blobs.writer().unwrap();
    std::io::Write::write_all(&mut writer, b"queued").unwrap();
    let size = writer.commit(&queued).unwrap();
    let mut producer = io_pimdir::PimdirProducer::open(dir, "test").unwrap();
    producer
        .enqueue(
            INBOX,
            &PimdirAction::Add {
                link_id: Some(ReplicaLinkId("mid:queued".into())),
                flags: ReplicaFlags::default(),
                object: Some(queued),
                meta: Some(ReplicaMeta(r#"{"v":1}"#.into())),
                handle: None,
            },
            Some(size),
            "2026-08-29T00:00:00.000Z",
        )
        .unwrap();
}

/// The rows a reconciliation must not disturb, whatever it adds back.
///
/// `object_hash` is not folded in, so the body every item points at is
/// the same before and after: an upgrade that lost one would be losing
/// the store's whole point.
#[derive(Debug, Eq, PartialEq)]
struct Spine {
    items: BTreeMap<(String, String), (i64, Option<String>, i64)>,
    bindings: BTreeSet<(String, String, String, String)>,
    objects: BTreeMap<String, i64>,
    queue: BTreeSet<(i64, String, Option<String>)>,
}

fn spine(conn: &Connection) -> Spine {
    let items = conn
        .prepare("SELECT collection, link_id, seq, object_hash, deleted FROM items")
        .unwrap()
        .query_map([], |row| {
            Ok((
                (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                (
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                ),
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let bindings = conn
        .prepare("SELECT collection, link_id, source, handle FROM bindings")
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let objects = conn
        .prepare("SELECT hash, size FROM objects")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let queue = conn
        .prepare("SELECT id, action, object_hash FROM queue")
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();

    Spine {
        items,
        bindings,
        objects,
        queue,
    }
}

/// Refuses a store the seeding left in a shape the reconciliation would
/// have nothing to say about.
///
/// Every assertion below is about rows surviving a shape change, so a
/// seeding that quietly stopped producing a conflict, a retained row or a
/// queued body would leave them all true over nothing.
fn assert_populated(conn: &Connection) {
    let count = |sql: &str| conn.query_row(sql, [], |row| row.get::<_, i64>(0)).unwrap();

    assert!(count("SELECT count(*) FROM collections WHERE account IS NOT NULL") >= 1);
    assert!(count("SELECT count(*) FROM items WHERE retained_at IS NOT NULL") >= 1);
    assert!(count("SELECT count(*) FROM items WHERE retained_at IS NULL") >= 2);
    assert!(count("SELECT count(*) FROM bindings WHERE conflicted = 1") >= 1);
    assert!(count("SELECT count(*) FROM bindings WHERE conflict_object IS NOT NULL") >= 1);
    assert!(count("SELECT count(*) FROM bindings WHERE shared_object IS NOT NULL") >= 2);
    assert!(count("SELECT count(*) FROM bindings WHERE base_present = 1") >= 1);
    assert!(count("SELECT count(*) FROM items WHERE sort_key != ''") >= 2);
    assert!(count("SELECT count(*) FROM queue WHERE object_hash IS NOT NULL") >= 1);
    assert!(count("SELECT count(DISTINCT source) FROM bindings") >= 2);

    // A backfill that did nothing would leave the column `NULL`, so at
    // least one binding must sit on an item that carries a body: the
    // check afterwards is otherwise satisfied by `NULL IS NULL`.
    assert!(
        count(
            "SELECT count(*) FROM bindings b JOIN items i \
               ON i.collection = b.collection AND i.link_id = b.link_id \
             WHERE i.object_hash IS NOT NULL"
        ) >= 2
    );
}

/// Rewrites a populated store into what an earlier draft would have
/// written, by taking `dropped`'s columns out.
///
/// The refcounts are recomputed afterwards, because an implementation
/// that never had a column never took the pin it justifies: leaving the
/// counts as they were would hand the reconciliation a drift it did not
/// cause and could not have prevented.
fn drop_columns(dir: &Path, dropped: &[(&str, &str)]) {
    let conn = Connection::open(dir.join("pimdir.db")).unwrap();
    for index in FOLDED_INDEXES {
        conn.execute_batch(&format!("DROP INDEX IF EXISTS {index}"))
            .unwrap();
    }
    for (table, column) in dropped {
        conn.execute_batch(&format!("ALTER TABLE {table} DROP COLUMN {column}"))
            .unwrap();
    }

    // NOTE: the canonical recompute names all five pointer columns, and
    // one of them is itself folded in, so the older store's own version
    // of it counts one term fewer.
    let mut terms = vec![
        "SELECT object_hash AS hash FROM items WHERE object_hash IS NOT NULL",
        "SELECT conflict_object FROM items WHERE conflict_object IS NOT NULL",
        "SELECT base_object FROM bindings WHERE base_object IS NOT NULL",
        "SELECT object_hash FROM queue WHERE object_hash IS NOT NULL",
    ];
    if columns(&conn, "bindings")
        .iter()
        .any(|c| c == "conflict_object")
    {
        terms.insert(
            3,
            "SELECT conflict_object FROM bindings WHERE conflict_object IS NOT NULL",
        );
    }
    conn.execute_batch(&format!(
        "UPDATE objects SET refcount = ( \
           SELECT count(*) FROM ({}) r WHERE r.hash = objects.hash)",
        terms.join(" UNION ALL ")
    ))
    .unwrap();
}

fn columns(conn: &Connection, table: &str) -> Vec<String> {
    conn.prepare(&format!("PRAGMA table_info({table})"))
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn index_names(conn: &Connection) -> BTreeSet<String> {
    conn.prepare("SELECT name FROM sqlite_schema WHERE type = 'index' AND name IS NOT NULL")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Opens a store whose shape lost `dropped`, and checks everything the
/// reconciliation owes it.
///
/// Returns nothing and asserts throughout: a caller enumerating subsets
/// wants the first failure to name the subset it was on, which the
/// messages do.
fn reconciles(dropped: &[(&str, &str)]) {
    let dir = tempfile::tempdir().unwrap();
    populate(dir.path());

    let seeded = Connection::open(dir.path().join("pimdir.db")).unwrap();
    assert_populated(&seeded);
    let before = spine(&seeded);
    let blobs_before: BTreeSet<String> = before.objects.keys().cloned().collect();
    drop(seeded);
    drop_columns(dir.path(), dropped);

    let store = PimdirStore::open(dir.path())
        .unwrap_or_else(|err| panic!("{dropped:?} refused the store: {err}"));

    // Every folded-in column is back, and so is every index over one.
    let conn = Connection::open(dir.path().join("pimdir.db")).unwrap();
    for (table, column) in FOLDED_IN {
        assert!(
            columns(&conn, table).contains(&column.to_string()),
            "{dropped:?}: {table}.{column} was not added back"
        );
    }
    let held = index_names(&conn);
    for index in FOLDED_INDEXES {
        assert!(
            held.contains(*index),
            "{dropped:?}: index {index} was not recreated"
        );
    }
    for (index, wanted) in sql::RESHAPED_INDEXES {
        let shape: Vec<String> = conn
            .prepare(&format!("PRAGMA index_info({index})"))
            .unwrap()
            .query_map([], |row| row.get::<_, String>(2))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(&shape, wanted, "{dropped:?}: index {index} has moved");
    }

    // The rows the reconciliation had no business touching.
    let after = spine(&conn);
    assert_eq!(after.items, before.items, "{dropped:?}: the items moved");
    assert_eq!(
        after.bindings, before.bindings,
        "{dropped:?}: the bindings moved"
    );
    assert_eq!(
        after.objects, before.objects,
        "{dropped:?}: the object index moved"
    );
    assert_eq!(after.queue, before.queue, "{dropped:?}: the queue moved");

    // Reclaiming is not reconciling: every body is still on disk.
    for hash in &blobs_before {
        assert!(
            dir.path()
                .join("objects")
                .join(&hash[0..2])
                .join(&hash[2..4])
                .join(hash)
                .is_file(),
            "{dropped:?}: body {hash} went missing"
        );
    }

    // §6's backfill: a binding left empty reads as never having folded,
    // and the first absorb after the upgrade files the source's own
    // pending edit as a cross-source divergence. The value the rows
    // already imply is the item's own body.
    if dropped.contains(&("bindings", "shared_object")) {
        let disagreeing: i64 = conn
            .query_row(
                "SELECT count(*) FROM bindings b JOIN items i \
                   ON i.collection = b.collection AND i.link_id = b.link_id \
                 WHERE b.shared_object IS NOT i.object_hash",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            disagreeing, 0,
            "{dropped:?}: shared_object was not backfilled from items.object_hash"
        );
    }

    // The reconciliation must leave the counts agreeing with the columns
    // it just changed, and every read must run.
    assert!(
        store.refcount_drift().unwrap().is_empty(),
        "{dropped:?}: the reconciliation left refcount drift"
    );
    store.list_collections().unwrap();
    store.list_accounts().unwrap();
    store.list_items(INBOX, None, 64).unwrap();
    store.list_items_page_asc(INBOX, None, 64).unwrap();
    store.list_items_page_desc(INBOX, None, 64).unwrap();
    store.list_retained(&INBOX.into(), None, 64).unwrap();
    store.count_retained(&INBOX.into()).unwrap();
    store.retained_bytes().unwrap();
    store.list_conflicts(None).unwrap();
    store.pending_actions(INBOX).unwrap();
    store.parked_actions().unwrap();
    store.item_bindings(INBOX, "mid:a").unwrap();
    store.load_hub(INBOX).unwrap();
    store.distinct_sources().unwrap();
    store.dangling().unwrap();

    // And the store still writes: a reconciled shape that only reads is
    // a store an upgrade cannot sync.
    let mut writing = PimdirStore::open(dir.path()).unwrap().for_source("left");
    writing
        .write(vec![
            object(&writing, b"after"),
            placement(
                &writing,
                INBOX,
                "after-1",
                "mid:after",
                Some(b"after"),
                Some(b"after"),
                ReplicaStatus::Clean,
                None,
            ),
        ])
        .unwrap();
    assert!(writing.refcount_drift().unwrap().is_empty());
}

/// The list this test drops from is the list the crate adds back, so a
/// fold added to one and not the other narrows this test to nothing.
#[test]
fn every_folded_in_column_is_covered() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("objects")).unwrap();
    PimdirStore::open(dir.path()).unwrap();
    let conn = Connection::open(dir.path().join("pimdir.db")).unwrap();

    // Every column this test claims is folded in exists in the current
    // schema; one renamed out from under the list would otherwise make
    // every drop below a silent no-op.
    for (table, column) in FOLDED_IN {
        assert!(
            columns(&conn, table).contains(&column.to_string()),
            "{table}.{column} is not in the current schema any more"
        );
    }

    // And the list is the crate's own, read out of the source that
    // declares it: spec §6 requires the folded-in set to be kept complete
    // as further columns are folded in, and a fold added there and not
    // here would narrow every case below without failing anything.
    let held: BTreeSet<(String, String)> = FOLDED_IN
        .iter()
        .map(|(table, column)| (table.to_string(), column.to_string()))
        .collect();
    assert_eq!(
        declared_folded_in(),
        held,
        "the crate's FOLDED_IN and this test's have drifted apart"
    );
}

/// The `(table, column)` pairs `reconcile_draft_shape` declares, read out
/// of src/client.rs.
///
/// The list is private, and duplicating it here is exactly the drift this
/// exists to catch, so it is parsed rather than restated: the const holds
/// nothing but string literals, three per entry.
fn declared_folded_in() -> BTreeSet<(String, String)> {
    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("client.rs"),
    )
    .unwrap();
    let start = source
        .find("const FOLDED_IN")
        .expect("FOLDED_IN is declared");
    let body = &source[start..];
    let end = body.find("];").expect("FOLDED_IN is terminated");

    let literals: Vec<&str> = body[..end].split('"').skip(1).step_by(2).collect();
    assert_eq!(literals.len() % 3, 0, "FOLDED_IN entries are triples");
    literals
        .chunks(3)
        .map(|entry| (entry[0].to_string(), entry[1].to_string()))
        .collect()
}

/// Each folded-in column on its own, so a failure names one column
/// instead of a subset.
#[test]
fn each_folded_in_column_is_reconciled_on_a_populated_store() {
    for column in FOLDED_IN {
        reconciles(std::slice::from_ref(column));
    }
}

/// The whole set at once: the shape of the first store the fold was
/// written against.
#[test]
fn a_store_missing_every_folded_in_column_is_reconciled() {
    reconciles(FOLDED_IN);
}

proptest! {
    #![proptest_config(Config {
        cases: 48,
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/draft_reconcile_populated.proptest-regressions",
        ))),
        ..Config::default()
    })]

    /// A real old store is missing several columns at once, and the
    /// reconciliation's paths interact: the backfill reads `items`, the
    /// index rebuild reads the columns it is about to recreate over, and
    /// the whole thing runs in one transaction.
    #[test]
    fn any_subset_of_the_folded_in_columns_is_reconciled(
        mask in proptest::collection::vec(any::<bool>(), FOLDED_IN.len())
    ) {
        let dropped: Vec<(&str, &str)> = FOLDED_IN
            .iter()
            .zip(&mask)
            .filter(|(_, keep)| **keep)
            .map(|(column, _)| *column)
            .collect();
        reconciles(&dropped);
    }
}
