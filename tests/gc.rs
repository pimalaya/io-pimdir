//! A store never collects itself (spec §5, §14).
//!
//! Reclamation lives in a collector rather than in the write path, so
//! the pattern §14 invites works: stream a body straight to its sharded
//! path, index it, attach it in a later batch. A write that swept its own
//! refcount-zero objects would destroy the body the second batch was
//! going to reference, silently and bytes included.

use std::fs;

use io_pimdir::{PimdirStore, sql};
use io_replica::{
    change::{ReplicaDropReason, ReplicaWriteOp},
    client::ReplicaStorage,
    collection::ReplicaCollectionId,
    object::{ReplicaHash, ReplicaObject},
    placement::{
        ReplicaBase, ReplicaFlags, ReplicaHandle, ReplicaLevel, ReplicaLinkId, ReplicaMeta,
        ReplicaPlacement, ReplicaStatus,
    },
};

fn inbox() -> ReplicaCollectionId {
    ReplicaCollectionId("INBOX".into())
}

fn placement(handle: &str, link: &str, hash: &str) -> ReplicaPlacement {
    ReplicaPlacement {
        sort_key: Default::default(),
        collection: inbox(),
        handle: ReplicaHandle(handle.into()),
        link_id: Some(ReplicaLinkId(link.into())),
        object: Some(ReplicaHash(hash.into())),
        level: ReplicaLevel::Full,
        meta: Some(ReplicaMeta("{\"v\":1}".into())),
        flags: ReplicaFlags::default(),
        status: ReplicaStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(ReplicaBase {
            flags: ReplicaFlags::default(),
            revision: None,
            object: Some(ReplicaHash(hash.into())),
        }),
        origin: None,
    }
}

fn store_object(hash: &str, body: &[u8]) -> ReplicaWriteOp {
    ReplicaWriteOp::StoreObject {
        object: ReplicaObject {
            hash: ReplicaHash(hash.into()),
            size: body.len(),
        },
        body: Some(body.to_vec()),
    }
}

fn blob_exists(dir: &std::path::Path, hash: &str) -> bool {
    dir.join("objects")
        .join(&hash[0..2])
        .join(&hash[2..4])
        .join(hash)
        .exists()
}

/// The pattern the collector exists for: bodies stored in one batch,
/// attached in the next.
#[test]
fn a_body_stored_without_a_placement_survives_until_it_is_attached() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("remote");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();

    // batch one indexes two bodies and attaches neither, which is what a
    // consumer streaming bodies ahead of their metadata does
    store
        .write(vec![
            store_object("cafebabe", b"first"),
            store_object("beef0000", b"second"),
        ])
        .unwrap();
    assert!(blob_exists(dir.path(), "cafebabe"));
    assert!(blob_exists(dir.path(), "beef0000"));

    // batch two attaches one of them, both having survived
    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(placement(
            "1", "mid:a", "cafebabe",
        ))])
        .unwrap();
    assert_eq!(
        store.list_items("INBOX", None, 10).unwrap()[0].object,
        Some(ReplicaHash("cafebabe".into()))
    );

    // the one nothing attached is what a collection is for
    let collected = store.collect_garbage().unwrap();
    assert_eq!((collected.objects, collected.blobs), (1, 1));
    assert_eq!(collected.bytes, 6);
    assert!(blob_exists(dir.path(), "cafebabe"), "still referenced");
    assert!(
        !blob_exists(dir.path(), "beef0000"),
        "referenced by nothing"
    );
}

/// The other half of what a collector takes: a file no row references,
/// which a crash between the blob write and the commit leaves behind.
#[test]
fn an_orphan_blob_is_collected_with_the_unreferenced_rows() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("remote");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    store
        .write(vec![
            store_object("cafebabe", b"body"),
            ReplicaWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe")),
        ])
        .unwrap();

    // a body on disk that no batch ever indexed
    let shard = dir.path().join("objects/de/adb");
    fs::create_dir_all(&shard).unwrap();
    fs::write(shard.join("deadbeef"), b"crashed").unwrap();

    // a half-written body belongs to a writer that has not committed, so
    // the collector leaves it alone
    fs::write(dir.path().join("objects/.tmp-1-1"), b"in flight").unwrap();

    let collected = store.collect_garbage().unwrap();
    assert_eq!((collected.objects, collected.blobs), (0, 1));
    assert_eq!(collected.bytes, 7);
    assert!(!shard.join("deadbeef").exists(), "the orphan is taken");
    assert!(blob_exists(dir.path(), "cafebabe"), "the live body is not");
    assert!(dir.path().join("objects/.tmp-1-1").exists(), "in flight");
}

/// A purge retires rows and releases what they pinned; the bytes are the
/// collector's to report.
#[test]
fn a_purge_retires_rows_and_the_collector_reclaims_the_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("remote");
    store
        .write(vec![
            store_object("cafebabe", b"body"),
            ReplicaWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe")),
        ])
        .unwrap();
    store
        .write(vec![ReplicaWriteOp::DropPlacement {
            collection: inbox(),
            handle: ReplicaHandle("1".into()),
            reason: ReplicaDropReason::Deleted,
        }])
        .unwrap();

    let purged = store
        .purge_retained_before("2100-01-01T00:00:00.000Z")
        .unwrap();
    assert_eq!(purged.items, 1);
    assert!(
        blob_exists(dir.path(), "cafebabe"),
        "released, not reclaimed"
    );

    let collected = store.collect_garbage().unwrap();
    assert_eq!(collected.bytes, 4);
    assert!(!blob_exists(dir.path(), "cafebabe"));
}

/// The repair `check --fix` runs: a drifted count settled from the
/// pointers that justify it, and the one dangling row it can clear.
#[test]
fn a_repair_recomputes_a_drifted_refcount_and_clears_a_dangling_binding() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("remote");
    store
        .write(vec![
            store_object("cafebabe", b"body"),
            ReplicaWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe")),
        ])
        .unwrap();

    // drift the count and dangle a binding the way a foreign writer
    // would: foreign keys are per connection and off by default, which is
    // how a store acquires a row the schema forbids
    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
    conn.execute("UPDATE objects SET refcount = 7", []).unwrap();
    conn.execute(
        "INSERT INTO bindings(collection, link_id, source, handle) \
         VALUES('INBOX', 'mid:gone', 'remote', 'x')",
        [],
    )
    .unwrap();
    drop(conn);

    assert_eq!(store.recompute_refcounts().unwrap(), 1);
    assert_eq!(store.clear_dangling_bindings().unwrap(), 1);

    // settled, not swept: the item still references the body, so the
    // repair leaves it at one and the collector finds nothing
    assert_eq!(store.collect_garbage().unwrap().blobs, 0);
    assert!(blob_exists(dir.path(), "cafebabe"));
    assert_eq!(
        store.recompute_refcounts().unwrap(),
        0,
        "nothing left to fix"
    );

    // and the statement is the canonical one, run against the real schema
    assert!(
        sql::ALL
            .iter()
            .any(|(name, _)| *name == "RECOMPUTE_REFCOUNTS")
    );
}
