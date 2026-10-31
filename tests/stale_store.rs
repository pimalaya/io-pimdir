//! A store missing a core table is refused by every role (spec §6): where
//! the owner open reconciles any other difference from the canonical
//! schema, a core table or trigger missing is a damaged store.
//!
//! The draft is edited in place and its version stamp stays at 1, so such
//! a store looks current to the version check alone. Every role opening it
//! says so instead of failing on the first statement that names the table.

#![cfg(feature = "client")]

use io_pimdir::client::{PimdirError, PimdirStore, producer::PimdirProducer, reader::PimdirReader};

#[test]
fn a_store_missing_a_canonical_table_is_refused_by_every_role() {
    let dir = tempfile::tempdir().unwrap();
    drop(PimdirStore::open(dir.path()).unwrap());

    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch("DROP TABLE item_address").unwrap();
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 1, "still stamped current, so only the shape says");
    drop(conn);

    let refused = PimdirStore::open(dir.path()).err().unwrap();
    assert!(matches!(
        refused,
        PimdirError::Stale {
            missing: "item_address"
        }
    ));
    assert_eq!(refused.code(), "store-stale");
    assert!(
        refused
            .to_string()
            .ends_with("run or upgrade the store's sync engine"),
        "{refused}"
    );
    assert!(matches!(
        PimdirReader::open(dir.path()),
        Err(PimdirError::Stale {
            missing: "item_address"
        })
    ));
    assert!(matches!(
        PimdirProducer::open(dir.path(), "test"),
        Err(PimdirError::Stale {
            missing: "item_address"
        })
    ));
}
