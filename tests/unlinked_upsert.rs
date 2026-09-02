//! An upsert carrying no link id, against a handle a binding already
//! holds (spec §10).
//!
//! A probe is unlinked until a `Meta` fetch resolves its identity, so the
//! store stages it apart from the hub. A handle the store has already
//! bound is not in that position: it names an item, and filing a second
//! row for it hands the engine two placements for one handle, which is
//! where a spurious `dup:` key comes from.

use io_pimdir::{PimdirError, PimdirSourceStore, PimdirStore};
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

/// A linked placement, the shape a resolved item is written back in.
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
        conflict_object: None,
        base: Some(ReplicaBase {
            flags: ReplicaFlags::default(),
            revision: Some("1".into()),
            object: None,
        }),
        origin: None,
    }
}

/// The freshly probed placement io-replica's `sync` builds for a remote
/// item it has no local side for: a handle, flags and a revision, and no
/// identity at all.
fn probed(handle: &str, revision: &str) -> ReplicaPlacement {
    ReplicaPlacement {
        sort_key: Default::default(),
        collection: inbox(),
        handle: ReplicaHandle(handle.into()),
        link_id: None,
        object: None,
        level: ReplicaLevel::Probed,
        meta: None,
        flags: ReplicaFlags::default(),
        status: ReplicaStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(ReplicaBase {
            flags: ReplicaFlags::default(),
            revision: Some(revision.into()),
            object: None,
        }),
        origin: None,
    }
}

fn projected(store: &PimdirSourceStore) -> Vec<ReplicaPlacement> {
    let mut placements = store
        .load(&inbox(), &ReplicaLoadScope::All)
        .unwrap()
        .placements;
    placements.sort_by(|a, b| a.handle.cmp(&b.handle));
    placements
}

fn opened(dir: &std::path::Path) -> PimdirSourceStore {
    let store = PimdirStore::open(dir).unwrap().for_source("remote");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    store
}

/// The floor: one handle, one row, whether or not the write naming it
/// carried the identity the store already holds for it.
#[test]
fn an_unlinked_upsert_lands_on_the_binding_its_handle_holds() {
    let dir = tempdir().unwrap();
    let mut store = opened(dir.path());

    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(placement(
            "u1", "msg-a",
        ))])
        .unwrap();

    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(probed("u1", "2"))])
        .unwrap();

    let placements = projected(&store);
    assert_eq!(
        placements.len(),
        1,
        "a bound handle answers with one placement, never two: {placements:?}",
    );
    assert_eq!(placements[0].handle, ReplicaHandle("u1".into()));
    assert_eq!(
        placements[0].link_id,
        Some(ReplicaLinkId("msg-a".into())),
        "and it is the item the handle was already bound to",
    );
}

/// The write that produces it: a remote edit resurrecting an item
/// deleted locally, which `sync` pulls as a fresh probe of the same
/// handle (io-replica, `pull_add`).
#[test]
fn a_resurrected_tombstone_stays_one_item() {
    let dir = tempdir().unwrap();
    let mut store = opened(dir.path());

    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(placement(
            "u1", "msg-a",
        ))])
        .unwrap();

    let mut tombstone = placement("u1", "msg-a");
    tombstone.status = ReplicaStatus::Tombstone;
    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(tombstone)])
        .unwrap();

    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(probed("u1", "2"))])
        .unwrap();

    let placements = projected(&store);
    assert_eq!(placements.len(), 1, "{placements:?}");
    assert_eq!(placements[0].link_id, Some(ReplicaLinkId("msg-a".into())));
    assert_eq!(
        placements[0].status,
        ReplicaStatus::Clean,
        "the pull resurrects the item rather than adding a second one",
    );
}

/// A handle nothing holds is what the residual is for: it stays
/// unlinked, waiting for the `Meta` upgrade that names it.
#[test]
fn a_probe_of_an_unbound_handle_stays_unlinked() {
    let dir = tempdir().unwrap();
    let mut store = opened(dir.path());

    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(probed("u1", "1"))])
        .unwrap();

    let placements = projected(&store);
    assert_eq!(placements.len(), 1);
    assert_eq!(placements[0].handle, ReplicaHandle("u1".into()));
    assert_eq!(
        placements[0].link_id, None,
        "a freshly probed row claims no identity",
    );
}

/// Keyed back onto its binding, an unlinked upsert is subject to the
/// rebind floor like any other: a batch claiming one identity under two
/// handles is refused rather than half applied.
#[test]
fn an_unlinked_upsert_is_seen_by_the_rebind_guard() {
    let dir = tempdir().unwrap();
    let mut store = opened(dir.path());

    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(placement(
            "u1", "msg-a",
        ))])
        .unwrap();

    let refused = store
        .write(vec![
            ReplicaWriteOp::UpsertPlacement(probed("u1", "2")),
            ReplicaWriteOp::UpsertPlacement(placement("u2", "msg-a")),
        ])
        .unwrap_err();

    assert!(
        matches!(&refused, PimdirError::Rebind { link_id, bound, incoming, .. }
            if link_id == "msg-a" && bound == "u1" && incoming == "u2"),
        "the refusal names both handles the batch claimed the key under: {refused}",
    );

    let placements = projected(&store);
    assert_eq!(placements.len(), 1);
    assert_eq!(placements[0].handle, ReplicaHandle("u1".into()));
}
