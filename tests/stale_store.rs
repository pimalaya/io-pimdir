//! A store an earlier draft wrote is refused, never reconciled (spec §6).
//!
//! The draft is edited in place and its version stamp stays at 1, so a
//! store missing a table the current schema declares looks current to the
//! version check alone. Every role opening it says so instead of failing
//! on the first statement that names the table.

use io_pimdir::client::{PimdirError, PimdirStore, producer::PimdirProducer, reader::PimdirReader};

#[test]
fn a_store_missing_a_canonical_table_is_refused_by_every_role() {
    let dir = tempfile::tempdir().unwrap();
    drop(PimdirStore::open(dir.path()).unwrap());

    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch("DROP TABLE probes").unwrap();
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 1, "still stamped current, so only the shape says");
    drop(conn);

    assert!(matches!(
        PimdirStore::open(dir.path()),
        Err(PimdirError::Stale { table: "probes" })
    ));
    assert!(matches!(
        PimdirReader::open(dir.path()),
        Err(PimdirError::Stale { table: "probes" })
    ));
    assert!(matches!(
        PimdirProducer::open(dir.path(), "test"),
        Err(PimdirError::Stale { table: "probes" })
    ));
}
