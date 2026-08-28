//! Reopening an existing store: the earlier-draft shape reconciliation, and
//! the schema-stamp agreement §4.2 requires (spec §6, the `draft` allowance).
//!
//! While the spec is a draft a column may be folded into `0001_init.sql`
//! rather than added as version 2, so an older store carries `user_version =
//! 1` and is not detectably out of date: the runner does nothing and the
//! missing column surfaces as a query error on the first read. §6 requires an
//! implementation to reconcile the shape on open or refuse the store, and this
//! crate reconciles.
//!
//! The older store is *derived* from the current schema rather than pasted in,
//! by dropping every folded-in column and the indexes over them, so the test
//! follows the next fold without being rewritten.

use std::fs;

use io_pimdir::{PimdirError, PimdirReader, PimdirStore, sql};
use rusqlite::Connection;
use tempfile::tempdir;

/// The columns folded into version 1 after it was first published, with the
/// indexes that name them. Mirrors `reconcile_draft_shape`'s own list.
const FOLDED_IN: &[(&str, &str)] = &[
    ("bindings", "conflicted"),
    ("bindings", "conflict_revision"),
    ("items", "retained_at"),
    ("items", "retained_by"),
    ("items", "sort_key"),
    ("collections", "account"),
    ("bindings", "base_present"),
];

/// The columns a later draft folded back out, with their declaration, so
/// a store written while one was in the schema can be recreated here.
/// Mirrors `reconcile_draft_shape`'s own list.
const FOLDED_OUT: &[(&str, &str, &str)] = &[("bindings", "ambiguous_handles", "TEXT")];

const FOLDED_INDEXES: &[&str] = &[
    "items_retained",
    "collections_by_account",
    "items_by_sort",
    "items_by_seq_global",
    "objects_garbage",
    "items_by_conflict_object",
    "queue_by_object",
    "bindings_by_handle",
];

#[test]
fn an_earlier_draft_store_is_reconciled_on_open() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("objects")).unwrap();

    let conn = Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch(sql::MIGRATION_0001).unwrap();
    conn.execute_batch(
        "INSERT INTO store_meta(id, version, hash_algo, created_at) VALUES(1, 1, 'blake3', '0')",
    )
    .unwrap();
    for index in FOLDED_INDEXES {
        conn.execute_batch(&format!("DROP INDEX {index}")).unwrap();
    }
    for (table, column) in FOLDED_IN {
        conn.execute_batch(&format!("ALTER TABLE {table} DROP COLUMN {column}"))
            .unwrap();
    }
    conn.pragma_update(None, "user_version", sql::VERSION)
        .unwrap();
    drop(conn);

    let store = PimdirStore::open(dir.path()).unwrap();
    store.ensure_collection("INBOX", "message/rfc822").unwrap();

    // Every read naming a folded-in column, which is what an unreconciled
    // store fails on: paging by link id, by sort key both ways, and the trash.
    assert!(store.list_items("INBOX", None, 10).unwrap().is_empty());
    assert!(
        store
            .list_items_page_asc("INBOX", None, 10)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .list_items_page_desc("INBOX", None, 10)
            .unwrap()
            .is_empty()
    );
    assert_eq!(store.count_retained(&"INBOX".into()).unwrap(), 0);
    assert!(store.list_accounts().unwrap().is_empty());
}

/// A column a later draft folded back *out* goes from an existing store on
/// open, the other half of the same allowance.
///
/// `bindings.ambiguous_handles` recorded the handles a source held one identity
/// under; the second copy is an item of its own now (spec §9), so the store
/// records no trace of an incoming handle at all. Left in place the column
/// would keep rows stating a rule the crate no longer has, and the store would
/// disagree with the canonical schema on the one table the write path is
/// strictest about.
#[test]
fn a_column_folded_back_out_is_dropped_on_open() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("objects")).unwrap();

    let conn = Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch(sql::MIGRATION_0001).unwrap();
    conn.execute_batch(
        "INSERT INTO store_meta(id, version, hash_algo, created_at) VALUES(1, 1, 'blake3', '0')",
    )
    .unwrap();
    for (table, column, decl) in FOLDED_OUT {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"))
            .unwrap();
    }
    conn.pragma_update(None, "user_version", sql::VERSION)
        .unwrap();
    drop(conn);

    let store = PimdirStore::open(dir.path()).unwrap();
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    drop(store);

    let conn = Connection::open(dir.path().join("pimdir.db")).unwrap();
    for (table, column, _) in FOLDED_OUT {
        let held: Vec<String> = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(
            !held.contains(&column.to_string()),
            "{table}.{column} is still there after the reconciliation"
        );
    }
}

/// An index an earlier draft created under a name the schema still uses, over
/// columns it no longer uses, is rebuilt rather than left alone.
///
/// `CREATE INDEX IF NOT EXISTS` keys on the name, so the ensure batch cannot
/// see the difference: it finds an index called `items_retained` and does
/// nothing, and the store goes on sorting every retained row of a collection to
/// return one page of the trash. Nothing errors, which is why this is checked.
#[test]
fn an_index_whose_columns_moved_is_rebuilt_on_open() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("objects")).unwrap();

    let conn = Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch(sql::MIGRATION_0001).unwrap();
    conn.execute_batch(
        "INSERT INTO store_meta(id, version, hash_algo, created_at) VALUES(1, 1, 'blake3', '0')",
    )
    .unwrap();
    // The shape an earlier draft wrote, under the name the current one keeps.
    conn.execute_batch(
        "DROP INDEX items_retained; \
         CREATE INDEX items_retained ON items(collection, retained_at) \
         WHERE retained_at IS NOT NULL",
    )
    .unwrap();
    conn.pragma_update(None, "user_version", sql::VERSION)
        .unwrap();
    drop(conn);

    let _store = PimdirStore::open(dir.path()).unwrap();

    let conn = Connection::open(dir.path().join("pimdir.db")).unwrap();
    for (index, wanted) in sql::RESHAPED_INDEXES {
        let held: Vec<String> = conn
            .prepare(&format!("PRAGMA index_info({index})"))
            .unwrap()
            .query_map([], |row| row.get::<_, String>(2))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(&held, wanted, "index {index} was not rebuilt");
    }
}

/// Spec §4.2: `PRAGMA user_version` and `store_meta.version` mirror one
/// another, and a store where they disagree is corrupt rather than a store at
/// either version. The stamps are written by two different statements, so one
/// landing without the other is exactly what a half-applied schema change
/// looks like.
#[test]
fn disagreeing_schema_stamps_are_refused() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("objects")).unwrap();

    // An owner-created store, then its store_meta stamp moved out from under
    // the pragma one.
    PimdirStore::open(dir.path()).unwrap();
    let conn = Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch("UPDATE store_meta SET version = 2 WHERE id = 1")
        .unwrap();
    drop(conn);

    assert!(matches!(
        PimdirStore::open(dir.path()),
        Err(PimdirError::VersionMismatch {
            user_version: 1,
            store_meta: 2
        })
    ));
    assert!(matches!(
        PimdirReader::open(dir.path()),
        Err(PimdirError::VersionMismatch { .. })
    ));
}

/// A foreign-key action is the half of the draft allowance reconciliation
/// cannot reach: `ALTER TABLE` adds a column but never an `ON UPDATE CASCADE`,
/// so §6's other branch applies and the store is refused with a message rather
/// than opened into a state where a rename fails one dependent row down.
#[test]
fn a_store_without_the_rename_cascades_is_refused() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("objects")).unwrap();

    // The current schema minus its cascades, which is the shape 0.2.0 created.
    let conn = Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch(&sql::MIGRATION_0001.replace("ON UPDATE CASCADE", ""))
        .unwrap();
    conn.execute_batch(
        "INSERT INTO store_meta(id, version, hash_algo, created_at) VALUES(1, 1, 'blake3', '0')",
    )
    .unwrap();
    conn.pragma_update(None, "user_version", sql::VERSION)
        .unwrap();
    drop(conn);

    assert!(matches!(
        PimdirStore::open(dir.path()),
        Err(PimdirError::Unreconcilable { .. })
    ));
    assert!(matches!(
        PimdirReader::open(dir.path()),
        Err(PimdirError::Unreconcilable { .. })
    ));
}
