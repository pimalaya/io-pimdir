//! What a binding last agreed with the hub on, across runs (spec §13).
//!
//! A binding holds two agreement points, one per axis. `base_object` is what
//! its source last agreed with its own remote, which only a sync moves, so it
//! stays behind while an unpushed edit waits and that gap is what makes the
//! push derivable. `shared_object` is what it last agreed with the shared item,
//! which every absorbed upsert moves.
//!
//! Reading the first as the second is what this exists to stop: a source's own
//! unpushed edit leaves the sync base behind the shared body exactly as another
//! source folding in does, so a second offline edit reads as two sources
//! disagreeing in a store that has one, and is dropped. The hub carries the
//! field in memory either way; the store is what makes it survive, and the
//! absorb that would file the conflict and the edit that settles it are
//! different runs.

use io_pimdir::{PimdirReader, PimdirSourceStore, PimdirStore};
use io_replica::{
    change::ReplicaWriteOp,
    client::ReplicaStorage,
    collection::ReplicaCollectionId,
    hub::ReplicaSourceId,
    object::{ReplicaHash, ReplicaObject},
    placement::{
        ReplicaBase, ReplicaFlags, ReplicaHandle, ReplicaLevel, ReplicaLinkId, ReplicaMeta,
        ReplicaPlacement, ReplicaStatus,
    },
    storage::ReplicaLoadScope,
};
use tempfile::tempdir;

fn contacts() -> ReplicaCollectionId {
    ReplicaCollectionId("contacts".into())
}

fn carddav() -> ReplicaSourceId {
    ReplicaSourceId("carddav".into())
}

/// One card: the body its source last synced, the body it holds now, and
/// whether its own merge left it conflicted.
fn card(
    base: &str,
    object: &str,
    status: ReplicaStatus,
    conflict_revision: Option<&str>,
) -> ReplicaPlacement {
    ReplicaPlacement {
        sort_key: Default::default(),
        collection: contacts(),
        handle: ReplicaHandle("card-a.vcf".into()),
        link_id: Some(ReplicaLinkId("card-a".into())),
        object: Some(ReplicaHash(object.into())),
        level: ReplicaLevel::Full,
        meta: Some(ReplicaMeta(r#"{"v":1}"#.into())),
        flags: ReplicaFlags::default(),
        status,
        conflict_revision: conflict_revision.map(Into::into),
        conflict_object: None,
        base: Some(ReplicaBase {
            flags: ReplicaFlags::default(),
            revision: Some("r-base".into()),
            object: Some(ReplicaHash(base.into())),
        }),
        origin: None,
    }
}

/// That card and the two bodies it names, as one batch: an object no
/// placement references is at refcount zero, so storing the bodies in a
/// batch of their own would leave the placement's foreign key dangling.
fn batch(
    base: &str,
    object: &str,
    status: ReplicaStatus,
    conflict_revision: Option<&str>,
) -> Vec<ReplicaWriteOp> {
    let mut ops = Vec::new();

    for hash in [base, object] {
        ops.push(ReplicaWriteOp::StoreObject {
            object: ReplicaObject {
                hash: ReplicaHash(hash.into()),
                size: hash.len(),
            },
            body: Some(hash.as_bytes().to_vec()),
        });
    }

    ops.push(ReplicaWriteOp::UpsertPlacement(card(
        base,
        object,
        status,
        conflict_revision,
    )));
    ops
}

fn seed(dir: &std::path::Path) -> PimdirSourceStore {
    let store = PimdirStore::open(dir).unwrap().for_source("carddav");
    store.ensure_collection("contacts", "text/vcard").unwrap();
    store
}

#[test]
fn the_agreement_point_survives_a_reopen() {
    let dir = tempdir().unwrap();
    let mut store = seed(dir.path());

    store
        .write(batch("bod0", "bod1", ReplicaStatus::Dirty, None))
        .unwrap();
    drop(store);

    let read = PimdirReader::open(dir.path()).unwrap();
    let bindings = read.item_bindings("contacts", "card-a").unwrap();
    let binding = &bindings[&carddav()];

    assert!(!binding.conflicted, "an ordinary binding, nothing special");
    assert_eq!(
        binding.base.as_ref().and_then(|base| base.object.clone()),
        Some(ReplicaHash("bod0".into())),
        "the sync base is what the source last agreed with its own remote, and \
         the pending push is derived from it staying behind",
    );
    assert_eq!(
        binding.shared_object,
        Some(ReplicaHash("bod1".into())),
        "and the agreement point is the shared body the reconcile settled on",
    );
}

/// The flag gates the conflict pair and nothing else. Gated on it too, the
/// agreement point would be erased at exactly the moment the edit resolving
/// the conflict needs it, and that edit would be dropped as a divergence.
#[test]
fn a_conflicted_binding_carries_one_too() {
    let dir = tempdir().unwrap();
    let mut store = seed(dir.path());

    store
        .write(batch(
            "bod0",
            "bod1",
            ReplicaStatus::Conflict,
            Some("r-remote"),
        ))
        .unwrap();
    drop(store);

    let read = PimdirReader::open(dir.path()).unwrap();
    let bindings = read.item_bindings("contacts", "card-a").unwrap();
    let binding = &bindings[&carddav()];

    assert!(binding.conflicted);
    assert_eq!(binding.conflict_revision.as_deref(), Some("r-remote"));
    assert_eq!(
        binding.shared_object,
        Some(ReplicaHash("bod1".into())),
        "the agreement point is not an exception and is not gated on the flag",
    );
}

/// The whole point, end to end: one source, no second source anywhere, and
/// two offline edits in two runs.
#[test]
fn a_second_offline_edit_across_a_restart_is_not_a_conflict() {
    let dir = tempdir().unwrap();
    let mut store = seed(dir.path());

    store
        .write(batch("bod0", "bod1", ReplicaStatus::Dirty, None))
        .unwrap();
    drop(store);

    // A new run, the push still pending, so the sync base is where the
    // first edit left it.
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("carddav");
    store
        .write(batch("bod0", "bod2", ReplicaStatus::Dirty, None))
        .unwrap();

    let loaded = store.load(&contacts(), &ReplicaLoadScope::All).unwrap();
    assert_eq!(
        loaded.placements[0].object,
        Some(ReplicaHash("bod2".into())),
        "the second edit is the item's body: measured from the sync base it \
         would read as another source having moved the shared one, and be \
         kept as the diverging body of a conflict nobody can resolve",
    );
    assert_eq!(
        loaded.placements[0].status,
        ReplicaStatus::Dirty,
        "and it is still waiting to be pushed, not conflicted",
    );
}

/// The draft allowance (spec §6): the column was folded into version 1 after
/// it was published, so a store written by an earlier draft is stamped
/// `user_version = 1` and lacks it.
///
/// Adding it back empty is not enough. A binding with no agreement point falls
/// back to the sync base, and a store carrying an unpushed edit has that base
/// behind the shared body by definition, so the first absorb after the upgrade
/// would file the source's own next edit as a divergence: one silent lost edit
/// per pending push, on the upgrade run.
#[test]
fn a_store_written_before_the_column_is_backfilled_from_the_item_body() {
    let dir = tempdir().unwrap();
    let mut store = seed(dir.path());

    store
        .write(batch("bod0", "bod1", ReplicaStatus::Dirty, None))
        .unwrap();
    drop(store);

    // Rewind to the earlier draft's shape. No index names this column, so it
    // drops on its own, where the conflict pair beside it needed two dropped
    // first.
    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch("ALTER TABLE bindings DROP COLUMN shared_object")
        .unwrap();
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 1, "still stamped current, so nothing flags it");
    drop(conn);

    // Opening reconciles the shape and backfills the column.
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("carddav");
    let bindings = store.item_bindings("contacts", "card-a").unwrap();
    assert_eq!(
        bindings[&carddav()].shared_object,
        Some(ReplicaHash("bod1".into())),
        "the upgraded store agrees with the body it holds, rather than reading \
         as a source that has never folded",
    );

    // Which is what the backfill is for: the edit after the upgrade is
    // adopted rather than filed against a shared body it never diverged from.
    store
        .write(batch("bod0", "bod2", ReplicaStatus::Dirty, None))
        .unwrap();
    let loaded = store.load(&contacts(), &ReplicaLoadScope::All).unwrap();
    assert_eq!(
        loaded.placements[0].object,
        Some(ReplicaHash("bod2".into())),
    );
}
