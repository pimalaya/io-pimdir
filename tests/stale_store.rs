//! A store naming this draft whose schema is not the canonical one is
//! damaged, and every profile refuses it (STORAGE §4.1, §6): drafts are
//! exact, so no difference is an earlier draft's to reconcile.
//!
//! The draft is edited in place and its version stamp stays at 1, so such
//! a store looks current to the marker and the stamps alone. Every profile
//! opening it says so, naming the object, instead of failing on the first
//! statement that reads it, and writes nothing.

#![cfg(feature = "client")]

use std::fs;

use io_pimdir::client::{PimdirError, PimdirStore, producer::PimdirProducer, reader::PimdirReader};

/// A store this crate created, then `sql` run on it.
fn damaged(sql: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    drop(PimdirStore::open(dir.path()).unwrap());
    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch(sql).unwrap();
    dir
}

fn refused_by_every_profile(dir: &std::path::Path, object: &str) {
    let before = fs::read(dir.join("pimdir.db")).unwrap();
    let refusals = [
        PimdirStore::open(dir).err(),
        PimdirStore::open_current(dir).err(),
        PimdirReader::open(dir).err(),
        PimdirProducer::open(dir, "test").err(),
    ];
    for refused in refusals {
        assert!(
            matches!(&refused, Some(PimdirError::Damaged { object: found }) if found == object),
            "{refused:?}"
        );
        assert_eq!(refused.unwrap().code(), "store-corrupt");
    }
    assert_eq!(fs::read(dir.join("pimdir.db")).unwrap(), before);
}

#[test]
fn a_store_missing_a_canonical_table_is_refused_by_every_profile() {
    let dir = damaged("DROP TABLE summary_value");
    refused_by_every_profile(dir.path(), "summary_value");
}

#[test]
fn a_column_dropped_is_a_damage_too() {
    let dir = damaged("ALTER TABLE summary DROP COLUMN title;");
    refused_by_every_profile(dir.path(), "summary");
}

#[test]
fn an_extra_index_is_a_damage_too() {
    let dir = damaged("CREATE INDEX zz_extra ON items(sort_key);");
    refused_by_every_profile(dir.path(), "zz_extra");
}

/// A canonical object missing is named before an extra one, wherever the
/// extra one sorts.
#[test]
fn a_missing_object_is_named_before_an_extra_one() {
    let dir = damaged(
        "CREATE INDEX aa_extra ON items(sort_key);
         DROP INDEX items_by_changed;",
    );
    refused_by_every_profile(dir.path(), "items_by_changed");
}

/// A draft's store holds version 1 alone (STORAGE §4.2): one whose two
/// stamps agree on another is damaged, not a later store.
#[test]
fn a_draft_store_at_another_schema_version_is_damaged() {
    let dir = damaged("PRAGMA user_version = 2; UPDATE store_meta SET version = 2;");
    refused_by_every_profile(dir.path(), "user_version");
}

/// A store of storage-draft-15 as it stood before objects were keyed by an
/// integer, its `objects` table keyed by the hash, is damaged: the draft
/// was edited in place, so its marker is this one's.
#[test]
fn a_store_keying_objects_by_hash_is_damaged() {
    let dir = damaged(
        "PRAGMA foreign_keys = OFF;
         DROP TABLE objects;
         CREATE TABLE objects (
             hash     TEXT PRIMARY KEY CHECK (typeof(hash) = 'text')
                      CHECK (length(hash) = 52 AND hash NOT GLOB '*[^a-z2-7]*'),
             size     INTEGER NOT NULL CHECK (typeof(size) = 'integer') CHECK (size >= 0),
             refcount INTEGER NOT NULL DEFAULT 0 CHECK (typeof(refcount) = 'integer') CHECK (refcount >= 0)
         );
         CREATE INDEX objects_garbage ON objects(refcount) WHERE refcount <= 0;",
    );
    refused_by_every_profile(dir.path(), "objects");
}
