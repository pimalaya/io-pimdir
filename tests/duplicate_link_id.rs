//! Persisting an identity a collection holds twice (spec §10).
//!
//! The engine freezes such an item, and the freeze is only worth anything if
//! it survives: the second copy appears in exactly one enumeration, and an
//! incremental one never mentions it again. That makes this the store's
//! contract, not the engine's alone.

use io_pimdir::{PimdirSourceStore, PimdirStore};
use io_replica::{
    change::ReplicaWriteOp,
    client::ReplicaStorage,
    collection::ReplicaCollectionId,
    placement::{
        ReplicaBase, ReplicaFlags, ReplicaHandle, ReplicaLevel, ReplicaLinkId, ReplicaMeta,
        ReplicaPlacement, ReplicaStatus,
    },
    storage::ReplicaLoadScope,
};
use tempfile::tempdir;

fn inbox() -> ReplicaCollectionId {
    ReplicaCollectionId("INBOX".into())
}

fn placement(handle: &str, link: &str) -> ReplicaPlacement {
    ReplicaPlacement {
        sort_key: Default::default(),
        collection: inbox(),
        handle: ReplicaHandle(handle.into()),
        link_id: Some(ReplicaLinkId(link.into())),
        object: None,
        level: ReplicaLevel::Meta,
        meta: Some(ReplicaMeta(r#"{"v":1}"#.into())),
        flags: ReplicaFlags::default(),
        status: ReplicaStatus::Clean,
        conflict_revision: None,
        base: Some(ReplicaBase {
            flags: ReplicaFlags::default(),
            revision: None,
            object: None,
        }),
        origin: None,
        ambiguous_handles: Vec::new(),
    }
}

fn projected(store: &PimdirSourceStore) -> Vec<ReplicaPlacement> {
    store
        .load(&inbox(), &ReplicaLoadScope::All)
        .unwrap()
        .placements
}

#[test]
fn ambiguous_handles_survive_a_reopen() {
    let dir = tempdir().unwrap();
    {
        let mut store = PimdirStore::open(dir.path()).unwrap().for_source("remote");
        store.ensure_collection("INBOX", "message/rfc822").unwrap();

        let mut frozen = placement("u1", "msg-a");
        frozen.status = ReplicaStatus::Ambiguous;
        frozen.ambiguous_handles = vec![ReplicaHandle("u2".into())];
        store
            .write(vec![ReplicaWriteOp::UpsertPlacement(frozen)])
            .unwrap();
    }

    // A fresh handle, as the next run of a daemon is.
    let store = PimdirStore::open(dir.path()).unwrap().for_source("remote");
    let placements = projected(&store);
    assert_eq!(placements.len(), 1);
    assert_eq!(
        placements[0].status,
        ReplicaStatus::Ambiguous,
        "a freeze that does not survive a restart forgets, and the item goes \
         back to being deletable on the next run",
    );
    assert_eq!(
        placements[0].ambiguous_handles,
        vec![ReplicaHandle("u2".into())],
    );
}

#[test]
fn a_write_never_repoints_a_binding_to_another_handle() {
    // The evidence used to be destroyed here, at the write: the second copy
    // repointed the binding and no later layer could tell the source held the
    // identity twice.
    let dir = tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("remote");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();

    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(placement(
            "u1", "msg-a",
        ))])
        .unwrap();
    // the same identity arriving under a different handle
    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(placement(
            "u2", "msg-a",
        ))])
        .unwrap();

    let placements = projected(&store);
    assert_eq!(placements.len(), 1);
    assert_eq!(
        placements[0].handle,
        ReplicaHandle("u1".into()),
        "the bound handle stays",
    );
    assert_eq!(
        placements[0].ambiguous_handles,
        vec![ReplicaHandle("u2".into())],
        "and the incoming one is recorded rather than swallowed",
    );
    assert_eq!(placements[0].status, ReplicaStatus::Ambiguous);
}

#[test]
fn an_ordinary_write_clears_nothing() {
    let dir = tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("remote");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();

    let mut frozen = placement("u1", "msg-a");
    frozen.status = ReplicaStatus::Ambiguous;
    frozen.ambiguous_handles = vec![ReplicaHandle("u2".into())];
    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(frozen.clone())])
        .unwrap();

    // a write naming the bound handle, carrying the freeze along
    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(frozen)])
        .unwrap();

    assert_eq!(
        projected(&store)[0].ambiguous_handles,
        vec![ReplicaHandle("u2".into())],
    );
}

#[test]
fn the_engine_clearing_the_freeze_clears_the_column() {
    let dir = tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("remote");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();

    let mut frozen = placement("u1", "msg-a");
    frozen.status = ReplicaStatus::Ambiguous;
    frozen.ambiguous_handles = vec![ReplicaHandle("u2".into())];
    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(frozen)])
        .unwrap();

    // the sync resolved it: the placement comes back clean, carrying none
    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(placement(
            "u1", "msg-a",
        ))])
        .unwrap();

    let placements = projected(&store);
    assert!(placements[0].ambiguous_handles.is_empty());
    assert_eq!(placements[0].status, ReplicaStatus::Clean);
}
