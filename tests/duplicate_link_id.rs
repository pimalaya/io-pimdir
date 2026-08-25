//! Persisting an identity a collection holds twice (spec §10).
//!
//! The engine freezes such an item, and the freeze is only worth anything if
//! it survives: the second copy appears in exactly one enumeration, and an
//! incremental one never mentions it again. That makes this the store's
//! contract, not the engine's alone.

use io_pimdir::{PimdirSourceStore, PimdirStore};
use io_replica::{
    change::{ReplicaDropReason, ReplicaWriteOp},
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

/// A rebuilt handle space is a repoint the floor above MUST let through.
///
/// A rekey drops the whole old spine and upserts every item under its new
/// handle, in one batch (spec §12). Read without knowing that, the two halves
/// are indistinguishable from the same source reporting one identity under a
/// second handle, and the floor keeps the old handle and records the new one:
/// a UIDVALIDITY bump then freezes every item of the collection, bound to
/// handles the server no longer has, with no way back.
///
/// What separates the two is the drop's reason. `Superseded` says the row is
/// being replaced, `Deleted` says the item went; only the first licenses a
/// repoint, and only for the handle it names.
#[test]
fn a_rekey_carries_the_binding_over_instead_of_freezing_it() {
    let dir = tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("remote");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();

    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(placement(
            "u1", "msg-a",
        ))])
        .unwrap();

    // The handle-space rebuild: the old spine goes, the same items come back
    // renumbered.
    store
        .write_rekeyed(
            "INBOX",
            vec![
                ReplicaWriteOp::DropPlacement {
                    collection: inbox(),
                    handle: ReplicaHandle("u1".into()),
                    reason: ReplicaDropReason::Superseded,
                },
                ReplicaWriteOp::UpsertPlacement(placement("101", "msg-a")),
            ],
        )
        .unwrap();

    let placements = projected(&store);
    assert_eq!(placements.len(), 1);
    assert_eq!(
        placements[0].handle,
        ReplicaHandle("101".into()),
        "the binding follows the rebuilt spine",
    );
    assert!(
        placements[0].ambiguous_handles.is_empty(),
        "and renumbering is not a duplicate",
    );
    assert_eq!(placements[0].status, ReplicaStatus::Clean);
}

/// The licence is per handle, not per batch.
///
/// A rekey batch that also carries a genuine second copy of one identity must
/// still freeze it: superseding `u1` says nothing about `u9`, and reading the
/// reason as a blanket permission would put the data loss back inside the one
/// operation that legitimately repoints.
#[test]
fn a_superseded_handle_licenses_only_its_own_repoint() {
    let dir = tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("remote");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();

    store
        .write(vec![
            ReplicaWriteOp::UpsertPlacement(placement("u1", "msg-a")),
            ReplicaWriteOp::UpsertPlacement(placement("u2", "msg-b")),
        ])
        .unwrap();

    store
        .write_rekeyed(
            "INBOX",
            vec![
                // msg-a is superseded and renumbered: carried over.
                ReplicaWriteOp::DropPlacement {
                    collection: inbox(),
                    handle: ReplicaHandle("u1".into()),
                    reason: ReplicaDropReason::Superseded,
                },
                ReplicaWriteOp::UpsertPlacement(placement("101", "msg-a")),
                // msg-b is not: this is the source holding it twice.
                ReplicaWriteOp::UpsertPlacement(placement("u9", "msg-b")),
            ],
        )
        .unwrap();

    let mut placements = projected(&store);
    placements.sort_by(|a, b| a.link_id.cmp(&b.link_id));
    assert_eq!(placements[0].handle, ReplicaHandle("101".into()));
    assert!(placements[0].ambiguous_handles.is_empty());

    assert_eq!(
        placements[1].handle,
        ReplicaHandle("u2".into()),
        "the untouched binding keeps its handle",
    );
    assert_eq!(
        placements[1].ambiguous_handles,
        vec![ReplicaHandle("u9".into())],
        "and the second copy is still recorded",
    );
    assert_eq!(placements[1].status, ReplicaStatus::Ambiguous);
}
