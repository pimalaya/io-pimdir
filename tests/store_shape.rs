//! A store names the shape its statements are written in (STORAGE §4.1):
//! `pimdir` for the canonical one. Every profile refuses a store of a
//! shape it does not implement, naming it, and writes nothing (§6 step 2).

#![cfg(feature = "client")]

use std::fs;

use io_pimdir::client::{PimdirError, PimdirStore, producer::PimdirProducer, reader::PimdirReader};

fn refused_as_shape(refused: Option<PimdirError>) {
    let refused = refused.expect("refused");
    assert!(
        matches!(&refused, PimdirError::Shape(shape) if shape == "android-sqlite"),
        "{refused:?}"
    );
    assert_eq!(refused.code(), "store-shape");
}

#[test]
fn a_store_of_another_shape_is_refused_by_every_profile() {
    let dir = tempfile::tempdir().unwrap();
    drop(PimdirStore::open(dir.path()).unwrap());

    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    let shape: String = conn
        .query_row("SELECT shape FROM store_meta", [], |r| r.get(0))
        .unwrap();
    assert_eq!(shape, "pimdir", "the canonical shape by default");
    conn.execute("UPDATE store_meta SET shape = 'android-sqlite'", [])
        .unwrap();
    drop(conn);
    let before = fs::read(dir.path().join("pimdir.db")).unwrap();

    refused_as_shape(PimdirStore::open(dir.path()).err());
    refused_as_shape(PimdirStore::open_current(dir.path()).err());
    refused_as_shape(PimdirReader::open(dir.path()).err());
    refused_as_shape(PimdirProducer::open(dir.path(), "test").err());

    assert_eq!(
        fs::read(dir.path().join("pimdir.db")).unwrap(),
        before,
        "nothing written"
    );
}
