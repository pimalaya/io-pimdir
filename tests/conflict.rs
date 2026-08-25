//! An unresolved per-source content conflict survives the store.
//!
//! The sync layer's memory of "this source and its own remote diverged" lives
//! in `bindings.conflicted` / `bindings.conflict_revision`. Without it the
//! merge re-derives on every run the push the remote already rejected, never
//! converging, and a client cannot tell which items need a human — so this is
//! about the state surviving a *reopen*, not just a round trip in memory.

use io_pimdir::{PimdirSourceStore, PimdirStore};
use io_replica::{
    change::ReplicaWriteOp,
    client::ReplicaStorage,
    collection::ReplicaCollectionId,
    object::{ReplicaHash, ReplicaObject},
    placement::{
        ReplicaBase, ReplicaFlags, ReplicaHandle, ReplicaLevel, ReplicaLinkId, ReplicaMeta,
        ReplicaPlacement, ReplicaStatus,
    },
    storage::ReplicaLoadScope,
};

fn contacts() -> ReplicaCollectionId {
    ReplicaCollectionId("contacts".into())
}

/// A placement of the one card, at `status`, with `conflict_revision`.
fn card(status: ReplicaStatus, conflict_revision: Option<&str>) -> ReplicaPlacement {
    ReplicaPlacement {
        sort_key: Default::default(),
        collection: contacts(),
        handle: ReplicaHandle("card1.vcf".into()),
        link_id: Some(ReplicaLinkId("uid:a".into())),
        object: Some(ReplicaHash("ed17".into())),
        level: ReplicaLevel::Full,
        meta: Some(ReplicaMeta(r#"{"v":1}"#.into())),
        flags: ReplicaFlags::default(),
        status,
        conflict_revision: conflict_revision.map(Into::into),
        base: Some(ReplicaBase {
            flags: ReplicaFlags::default(),
            revision: Some("r-base".into()),
            object: Some(ReplicaHash("0rig".into())),
        }),
        origin: None,
        ambiguous_handles: Vec::new(),
    }
}

fn seed(dir: &std::path::Path) -> PimdirSourceStore {
    let store = PimdirStore::open(dir).unwrap().for_source("left");
    store.ensure_collection("contacts", "text/vcard").unwrap();
    store
}

/// A placement plus the two bodies it points at, as one batch.
///
/// They must ride in the **same** batch: an object no placement references has
/// refcount 0 and is swept at the end of its own batch, so storing them
/// separately would leave the placement's foreign key dangling.
fn card_batch(status: ReplicaStatus, conflict_revision: Option<&str>) -> Vec<ReplicaWriteOp> {
    vec![
        ReplicaWriteOp::StoreObject {
            object: ReplicaObject {
                hash: ReplicaHash("0rig".into()),
                size: 3,
            },
            body: Some(b"old".to_vec()),
        },
        ReplicaWriteOp::StoreObject {
            object: ReplicaObject {
                hash: ReplicaHash("ed17".into()),
                size: 3,
            },
            body: Some(b"new".to_vec()),
        },
        ReplicaWriteOp::UpsertPlacement(card(status, conflict_revision)),
    ]
}

#[test]
fn a_conflict_survives_a_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seed(dir.path());

    store
        .write(card_batch(ReplicaStatus::Conflict, Some("r-remote")))
        .unwrap();
    drop(store);

    // Reopened from disk: the merge must still see the conflict, or it
    // re-derives the push the remote already rejected.
    let store = PimdirStore::open(dir.path()).unwrap().for_source("left");
    let loaded = store.load(&contacts(), &ReplicaLoadScope::All).unwrap();
    assert_eq!(loaded.placements.len(), 1);
    assert_eq!(loaded.placements[0].status, ReplicaStatus::Conflict);
    assert_eq!(
        loaded.placements[0].conflict_revision.as_deref(),
        Some("r-remote"),
        "the observed remote revision is what a resolver merges against"
    );
}

#[test]
fn resolving_the_conflict_clears_it_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seed(dir.path());

    store
        .write(card_batch(ReplicaStatus::Conflict, Some("r-remote")))
        .unwrap();
    // The consumer resolves with an ordinary edit — no dedicated call.
    store.write(card_batch(ReplicaStatus::Dirty, None)).unwrap();
    drop(store);

    let store = PimdirStore::open(dir.path()).unwrap().for_source("left");
    let loaded = store.load(&contacts(), &ReplicaLoadScope::All).unwrap();
    assert_ne!(loaded.placements[0].status, ReplicaStatus::Conflict);
    assert_eq!(
        loaded.placements[0].conflict_revision, None,
        "a resolved binding must not carry a stale revision forward"
    );
}

#[test]
fn a_store_from_an_earlier_draft_of_v1_is_reconciled_on_open() {
    // The draft allowance (spec §6): the two columns were folded into version 1
    // after it was published, so a store written by an earlier draft is stamped
    // `user_version = 1` yet lacks them. It must be healed on open, not left to
    // fail on the next query.
    let dir = tempfile::tempdir().unwrap();
    let store = PimdirStore::open(dir.path()).unwrap();
    drop(store);

    // Rewind the store to the earlier draft's shape.
    let db = dir.path().join("pimdir.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("ALTER TABLE bindings DROP COLUMN conflicted")
        .unwrap();
    conn.execute_batch("ALTER TABLE bindings DROP COLUMN conflict_revision")
        .unwrap();
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 1, "still stamped current, so nothing flags it");
    drop(conn);

    // Opening heals it, and the store works.
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("left");
    store.ensure_collection("contacts", "text/vcard").unwrap();
    store
        .write(card_batch(ReplicaStatus::Conflict, Some("r-remote")))
        .unwrap();

    let loaded = store.load(&contacts(), &ReplicaLoadScope::All).unwrap();
    assert_eq!(loaded.placements[0].status, ReplicaStatus::Conflict);
    assert_eq!(
        loaded.placements[0].conflict_revision.as_deref(),
        Some("r-remote")
    );

    // Idempotent: a second open of a current store changes nothing.
    drop(store);
    let store = PimdirStore::open(dir.path()).unwrap().for_source("left");
    assert_eq!(
        store
            .load(&contacts(), &ReplicaLoadScope::All)
            .unwrap()
            .placements[0]
            .status,
        ReplicaStatus::Conflict
    );
}
