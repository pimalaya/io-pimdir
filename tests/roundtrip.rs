//! Single-source round-trip (write → reopen → load → lookup → drop → GC), and
//! two-source propagation (a copy appears on the other side; a delete lingers as
//! a tombstone) — the N=1 and N=2 cases of the same store.

use std::path::Path;

use io_pimdir::PimdirStore;
use io_replica::{
    change::ReplicaWriteOp,
    client::ReplicaStorage,
    collection::{ReplicaCheckpoint, ReplicaCollectionId},
    object::{ReplicaHash, ReplicaObject},
    placement::{
        ReplicaBase, ReplicaFlags, ReplicaHandle, ReplicaLevel, ReplicaLinkId, ReplicaPlacement,
        ReplicaStatus,
    },
};

fn inbox() -> ReplicaCollectionId {
    ReplicaCollectionId("INBOX".into())
}

/// A hydrated, linked placement with a matching base (so it projects clean).
fn placement(handle: &str, link: &str, hash: &str, flags: &[&str]) -> ReplicaPlacement {
    let flags = ReplicaFlags::from_iter(flags.iter().copied());
    ReplicaPlacement {
        collection: inbox(),
        handle: ReplicaHandle(handle.into()),
        link_id: Some(ReplicaLinkId(link.into())),
        object: Some(ReplicaHash(hash.into())),
        level: ReplicaLevel::Full,
        meta: None,
        flags: flags.clone(),
        status: ReplicaStatus::Clean,
        conflict_revision: None,
        base: Some(ReplicaBase {
            flags,
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
        body: body.to_vec(),
    }
}

fn blob_exists(dir: &Path, hash: &str) -> bool {
    dir.join("objects")
        .join(&hash[0..2])
        .join(&hash[2..4])
        .join(hash)
        .exists()
}

#[test]
fn single_source_write_reopen_lookup_and_gc() {
    let dir = tempfile::tempdir().unwrap();

    let mut store = PimdirStore::open(dir.path(), "local").unwrap();
    store
        .write(vec![
            store_object("cafebabe", b"abc"),
            ReplicaWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe", &["\\Seen"])),
            ReplicaWriteOp::SetCheckpoint {
                collection: inbox(),
                checkpoint: ReplicaCheckpoint(vec![1, 2, 3]),
            },
        ])
        .unwrap();
    assert!(blob_exists(dir.path(), "cafebabe"), "blob written");

    // Reopen: the item, its flags, its body and the checkpoint all survive.
    drop(store);
    let mut store = PimdirStore::open(dir.path(), "local").unwrap();
    let loaded = store.load(&inbox()).unwrap();
    assert_eq!(loaded.placements.len(), 1);
    assert_eq!(
        loaded.placements[0].object,
        Some(ReplicaHash("cafebabe".into()))
    );
    assert!(loaded.placements[0].flags.0.iter().any(|f| f == "\\Seen"));
    assert_eq!(loaded.checkpoint, Some(ReplicaCheckpoint(vec![1, 2, 3])));

    let known = store
        .lookup_objects(&[ReplicaLinkId("mid:a".into())])
        .unwrap();
    assert_eq!(
        known.get(&ReplicaLinkId("mid:a".into())),
        Some(&ReplicaHash("cafebabe".into()))
    );

    // Dropping the only binding removes the item and GCs its orphan blob.
    store
        .write(vec![ReplicaWriteOp::DropPlacement {
            collection: inbox(),
            handle: ReplicaHandle("1".into()),
        }])
        .unwrap();
    assert!(store.load(&inbox()).unwrap().placements.is_empty());
    assert!(!blob_exists(dir.path(), "cafebabe"), "orphan blob GC'd");
}

#[test]
fn collection_kind_is_declared_and_survives_a_lazy_write() {
    let dir = tempfile::tempdir().unwrap();
    let store = PimdirStore::open(dir.path(), "local").unwrap();

    // Unknown until declared.
    assert_eq!(store.collection_kind("INBOX").unwrap(), None);

    // Declaring creates the collection with its media type.
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    assert_eq!(
        store.collection_kind("INBOX").unwrap().as_deref(),
        Some("message/rfc822")
    );

    // A write's lazy `ENSURE_COLLECTION` must not clobber the declared kind.
    let mut store = store;
    store
        .write(vec![ReplicaWriteOp::SetCheckpoint {
            collection: inbox(),
            checkpoint: ReplicaCheckpoint(vec![9]),
        }])
        .unwrap();
    assert_eq!(
        store.collection_kind("INBOX").unwrap().as_deref(),
        Some("message/rfc822"),
        "a sync write preserves the declared kind"
    );

    // Redeclaring updates it (e.g. a store reused for another kind).
    store.ensure_collection("INBOX", "text/vcard").unwrap();
    assert_eq!(
        store.collection_kind("INBOX").unwrap().as_deref(),
        Some("text/vcard")
    );
}

#[test]
fn two_source_copy_and_delete_propagation() {
    let dir = tempfile::tempdir().unwrap();
    let mut left = PimdirStore::open(dir.path(), "left").unwrap();
    let mut right = PimdirStore::open(dir.path(), "right").unwrap();

    // The item lands on the left source only.
    left.write(vec![
        store_object("cafebabe", b"abc"),
        ReplicaWriteOp::UpsertPlacement(placement("L1", "mid:a", "cafebabe", &["\\Seen"])),
    ])
    .unwrap();

    // Right doesn't hold it yet, but the body is hydrated, so right projects a
    // Created copy — the cross-source propagation.
    let right_view = right.load(&inbox()).unwrap();
    assert_eq!(right_view.placements.len(), 1);
    assert_eq!(right_view.placements[0].status, ReplicaStatus::Created);
    assert_eq!(
        right_view.placements[0].object,
        Some(ReplicaHash("cafebabe".into()))
    );

    // Right accepts the copy (binds its own handle).
    right
        .write(vec![ReplicaWriteOp::UpsertPlacement(placement(
            "R1",
            "mid:a",
            "cafebabe",
            &["\\Seen"],
        ))])
        .unwrap();

    // Left deletes it. The item lingers (deleted) because right still holds it.
    left.write(vec![ReplicaWriteOp::DropPlacement {
        collection: inbox(),
        handle: ReplicaHandle("L1".into()),
    }])
    .unwrap();

    // Right now sees a tombstone — the delete propagated as a pending remove.
    let right_view = right.load(&inbox()).unwrap();
    assert_eq!(right_view.placements.len(), 1);
    assert_eq!(right_view.placements[0].status, ReplicaStatus::Tombstone);
    // Left no longer holds it, so it projects nothing (not a re-copy).
    assert!(left.load(&inbox()).unwrap().placements.is_empty());
}
