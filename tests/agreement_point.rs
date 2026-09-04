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

use io_pimdir::{
    change::PimdirWriteOp,
    collection::PimdirCollectionId,
    hub::PimdirSourceId,
    load::PimdirLoadScope,
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
};
use io_pimdir::{
    client::reader::PimdirReader,
    client::{PimdirSourceStore, PimdirStore},
};
use tempfile::tempdir;

fn contacts() -> PimdirCollectionId {
    PimdirCollectionId("contacts".into())
}

fn carddav() -> PimdirSourceId {
    PimdirSourceId("carddav".into())
}

/// One card: the body its source last synced, the body it holds now, and
/// whether its own merge left it conflicted.
fn card(
    base: &str,
    object: &str,
    status: PimdirStatus,
    conflict_revision: Option<&str>,
) -> PimdirPlacement {
    PimdirPlacement {
        sort_key: Default::default(),
        collection: contacts(),
        handle: PimdirHandle("card-a.vcf".into()),
        link_id: Some(PimdirLinkId("card-a".into())),
        object: Some(PimdirHash(object.into())),
        level: PimdirLevel::Full,
        summary: None,
        flags: PimdirFlags::default(),
        status,
        conflict_revision: conflict_revision.map(Into::into),
        conflict_object: None,
        base: Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: Some("r-base".into()),
            object: Some(PimdirHash(base.into())),
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
    status: PimdirStatus,
    conflict_revision: Option<&str>,
) -> Vec<PimdirWriteOp> {
    let mut ops = Vec::new();

    for hash in [base, object] {
        ops.push(PimdirWriteOp::StoreObject {
            object: PimdirObject {
                hash: PimdirHash(hash.into()),
                size: hash.len(),
            },
            body: Some(hash.as_bytes().to_vec()),
        });
    }

    ops.push(PimdirWriteOp::UpsertPlacement(card(
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
        .write(batch("bod0", "bod1", PimdirStatus::Dirty, None))
        .unwrap();
    drop(store);

    let read = PimdirReader::open(dir.path()).unwrap();
    let bindings = read.item_bindings("contacts", "card-a").unwrap();
    let binding = &bindings[&carddav()];

    assert!(!binding.conflicted, "an ordinary binding, nothing special");
    assert_eq!(
        binding.base.as_ref().and_then(|base| base.object.clone()),
        Some(PimdirHash("bod0".into())),
        "the sync base is what the source last agreed with its own remote, and \
         the pending push is derived from it staying behind",
    );
    assert_eq!(
        binding.shared_object,
        Some(PimdirHash("bod1".into())),
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
            PimdirStatus::Conflict,
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
        Some(PimdirHash("bod1".into())),
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
        .write(batch("bod0", "bod1", PimdirStatus::Dirty, None))
        .unwrap();
    drop(store);

    // A new run, the push still pending, so the sync base is where the
    // first edit left it.
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("carddav");
    store
        .write(batch("bod0", "bod2", PimdirStatus::Dirty, None))
        .unwrap();

    let loaded = store.load(&contacts(), &PimdirLoadScope::All).unwrap();
    assert_eq!(
        loaded.placements[0].object,
        Some(PimdirHash("bod2".into())),
        "the second edit is the item's body: measured from the sync base it \
         would read as another source having moved the shared one, and be \
         kept as the diverging body of a conflict nobody can resolve",
    );
    assert_eq!(
        loaded.placements[0].status,
        PimdirStatus::Dirty,
        "and it is still waiting to be pushed, not conflicted",
    );
}
