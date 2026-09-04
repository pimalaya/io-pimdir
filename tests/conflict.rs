//! An unresolved per-source content conflict survives the store.
//!
//! The sync layer's memory of "this source and its own remote diverged" lives
//! in `bindings.conflicted`, `bindings.conflict_revision` and
//! `bindings.conflict_object`. Without it the merge re-derives on every run the
//! push the remote already rejected, never converging, and a client cannot tell
//! which items need a human, so this is about the state surviving a *reopen*,
//! not just a round trip in memory.
//!
//! The body has a second requirement the revision does not: it has to outlive
//! the collector. Resolution is a person's decision, taken days after the run
//! that found the divergence, and a body swept in between leaves a revision
//! naming bytes nobody holds.

use io_pimdir::client::{PimdirSourceStore, PimdirStore};
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

fn contacts() -> PimdirCollectionId {
    PimdirCollectionId("contacts".into())
}

/// A placement of one card: the body it carries, where it sits, and the
/// conflict pair its binding persists.
///
/// The body is a parameter because a resolution is an ordinary edit
/// carrying the merged card: written at the body the conflict was filed
/// at, the second write differs from the first in nothing but its status,
/// and every assertion about what a resolution does to the stored body
/// holds trivially.
fn card(
    collection: &str,
    handle: &str,
    link: &str,
    object: &str,
    status: PimdirStatus,
    conflict_revision: Option<&str>,
    conflict_object: Option<&str>,
) -> PimdirPlacement {
    PimdirPlacement {
        sort_key: Default::default(),
        collection: PimdirCollectionId(collection.into()),
        handle: PimdirHandle(handle.into()),
        link_id: Some(PimdirLinkId(link.into())),
        object: Some(PimdirHash(object.into())),
        level: PimdirLevel::Full,
        summary: None,
        flags: PimdirFlags::default(),
        status,
        conflict_revision: conflict_revision.map(Into::into),
        conflict_object: conflict_object.map(|hash| PimdirHash(hash.into())),
        base: Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: Some("r-base".into()),
            object: Some(PimdirHash("0rig".into())),
        }),
        origin: None,
    }
}

fn store_object(hash: &str, body: &[u8]) -> PimdirWriteOp {
    PimdirWriteOp::StoreObject {
        object: PimdirObject {
            hash: PimdirHash(hash.into()),
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

fn seed(dir: &std::path::Path) -> PimdirSourceStore {
    let store = PimdirStore::open(dir).unwrap().for_source("left");
    store.ensure_collection("contacts", "text/vcard").unwrap();
    store
}

/// The bytes filed under one hash, distinct per hash so a body read back
/// out of the store says which one it is.
fn body_of(hash: &str) -> Vec<u8> {
    format!("the card at {hash}").into_bytes()
}

/// A placement plus the bodies it points at, as one batch.
///
/// They must ride in the **same** batch: an object no placement references has
/// refcount 0 and is swept at the end of its own batch, so storing them
/// separately would leave the placement's foreign key dangling.
fn card_batch(
    object: &str,
    status: PimdirStatus,
    conflict_revision: Option<&str>,
    conflict_object: Option<&str>,
) -> Vec<PimdirWriteOp> {
    let mut batch = vec![
        store_object("0rig", b"old"),
        store_object(object, &body_of(object)),
    ];

    if let Some(hash) = conflict_object {
        batch.push(store_object(hash, b"remote"));
    }

    batch.push(PimdirWriteOp::UpsertPlacement(card(
        "contacts",
        "card1.vcf",
        "uid:a",
        object,
        status,
        conflict_revision,
        conflict_object,
    )));
    batch
}

#[test]
fn a_conflict_survives_a_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seed(dir.path());

    store
        .write(card_batch(
            "ed17",
            PimdirStatus::Conflict,
            Some("r-remote"),
            Some("rmte"),
        ))
        .unwrap();
    drop(store);

    // Reopened from disk: the merge must still see the conflict, or it
    // re-derives the push the remote already rejected.
    let store = PimdirStore::open(dir.path()).unwrap().for_source("left");
    let loaded = store.load(&contacts(), &PimdirLoadScope::All).unwrap();
    assert_eq!(loaded.placements.len(), 1);
    assert_eq!(loaded.placements[0].status, PimdirStatus::Conflict);
    assert_eq!(
        loaded.placements[0].conflict_revision.as_deref(),
        Some("r-remote"),
        "the observed remote revision is what a resolver merges against"
    );
    assert_eq!(
        loaded.placements[0].conflict_object,
        Some(PimdirHash("rmte".into())),
        "and the body at that revision is what it merges, with no remote to ask"
    );
}

/// A resolution moves the item's body, and that is the half of it the
/// store has to make durable.
///
/// Clearing the flags is the visible half. the hub requires the
/// resolving edit to be adopted as the shared body too, because a binding
/// cleared of its conflict while the item still holds the body the merge
/// replaced leaves the next run pushing the unmerged body over the remote
/// the merge was made against. That is `items.object_hash` moving, and
/// this is the only place a resolution is run through the store, so a
/// resolution written at the body the conflict was filed at would assert
/// the flags and nothing else.
#[test]
fn resolving_the_conflict_clears_it_and_adopts_the_merged_body() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seed(dir.path());

    store
        .write(card_batch(
            "ed17",
            PimdirStatus::Conflict,
            Some("r-remote"),
            Some("rmte"),
        ))
        .unwrap();
    // The consumer resolves with an ordinary edit, carrying the merged
    // card: no dedicated call, and a body neither side held before.
    store
        .write(card_batch("mrgd", PimdirStatus::Dirty, None, None))
        .unwrap();
    drop(store);

    let store = PimdirStore::open(dir.path()).unwrap().for_source("left");
    let loaded = store.load(&contacts(), &PimdirLoadScope::All).unwrap();
    assert_ne!(loaded.placements[0].status, PimdirStatus::Conflict);
    assert_eq!(
        loaded.placements[0].conflict_revision, None,
        "a resolved binding must not carry a stale revision forward"
    );
    assert_eq!(
        loaded.placements[0].conflict_object, None,
        "nor the body that revision named, which the remote may have replaced"
    );

    assert_eq!(
        loaded.placements[0].object,
        Some(PimdirHash("mrgd".into())),
        "the merged card is what the store holds; keeping the pre-merge body \
         discards the resolution and pushes it over the remote next run"
    );
    assert_eq!(
        store.blobs().get(&PimdirHash("mrgd".into())).unwrap(),
        Some(body_of("mrgd")),
        "and the hash it holds resolves to the merged bytes"
    );
    assert_eq!(
        store.list_items("contacts", None, 10).unwrap()[0].object,
        Some(PimdirHash("mrgd".into())),
        "and the client read agrees with the seam about which body it is"
    );
}

/// The pin: a body kept only by a conflicted binding survives the sweep.
///
/// Nothing else references it. The item's own body is the local side of the
/// divergence and the base is the ancestor, so without the binding's reference
/// the remote side is at refcount zero from the moment it lands, and the first
/// collection after the run that found the conflict takes it.
#[test]
fn a_conflict_body_outlives_a_collection() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seed(dir.path());

    store
        .write(card_batch(
            "ed17",
            PimdirStatus::Conflict,
            Some("r-remote"),
            Some("rmte"),
        ))
        .unwrap();
    assert!(blob_exists(dir.path(), "rmte"));

    let collected = store.collect_garbage().unwrap();
    assert_eq!(collected.objects, 0, "every body is referenced");
    assert!(
        blob_exists(dir.path(), "rmte"),
        "the divergence a person has not looked at yet is still readable"
    );

    let loaded = store.load(&contacts(), &PimdirLoadScope::All).unwrap();
    assert_eq!(
        loaded.placements[0].conflict_object,
        Some(PimdirHash("rmte".into()))
    );
}

/// The other half: the pin is a pin, not a leak, and neither is the body
/// the resolution superseded.
///
/// A resolution is an ordinary edit, so it clears the binding's conflict
/// and with it the only reference to the remote body, and it repoints the
/// item at the merged one, which releases the body the merge replaced.
/// Both are unreferenced afterwards and the next collection takes both;
/// written at one body throughout, the second release does not exist to
/// be observed.
#[test]
fn resolving_releases_the_pin_and_the_next_collection_takes_the_body() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seed(dir.path());

    store
        .write(card_batch(
            "ed17",
            PimdirStatus::Conflict,
            Some("r-remote"),
            Some("rmte"),
        ))
        .unwrap();
    store
        .write(card_batch("mrgd", PimdirStatus::Dirty, None, None))
        .unwrap();
    assert!(
        store.refcount_drift().unwrap().is_empty(),
        "the two releases the resolution made are counted"
    );

    let collected = store.collect_garbage().unwrap();
    assert_eq!(
        (collected.objects, collected.blobs),
        (2, 2),
        "the diverging body and the one the merge replaced are both released"
    );
    assert!(
        !blob_exists(dir.path(), "rmte"),
        "a resolved conflict holds nothing"
    );
    assert!(
        !blob_exists(dir.path(), "ed17"),
        "and the pre-merge body is released rather than pinned for ever"
    );
    assert_eq!(
        store.blobs().get(&PimdirHash("mrgd".into())).unwrap(),
        Some(body_of("mrgd")),
        "the merged body is what survives the collection"
    );
    assert!(blob_exists(dir.path(), "0rig"), "and so does the base");
}

/// What is waiting for a decision, asked of the store rather than assembled
/// by paging every collection.
#[test]
fn the_listing_names_every_conflicted_binding_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seed(dir.path());
    store
        .ensure_collection("calendar", "text/calendar")
        .unwrap();

    // Two collections, each holding one conflicted item and one clean one, so
    // a listing that ignored the flag or stopped at the first collection would
    // come back with the wrong rows either way.
    for (collection, handle, link, status, revision, object) in [
        (
            "contacts",
            "card1.vcf",
            "uid:a",
            PimdirStatus::Conflict,
            Some("r-remote"),
            Some("rmte"),
        ),
        (
            "contacts",
            "card2.vcf",
            "uid:b",
            PimdirStatus::Clean,
            None,
            None,
        ),
        (
            "calendar",
            "event1.ics",
            "uid:c",
            PimdirStatus::Conflict,
            Some("r-event"),
            Some("evnt"),
        ),
        (
            "calendar",
            "event2.ics",
            "uid:d",
            PimdirStatus::Clean,
            None,
            None,
        ),
    ] {
        let mut batch = vec![
            store_object("0rig", b"old"),
            store_object("ed17", &body_of("ed17")),
        ];
        if let Some(hash) = object {
            batch.push(store_object(hash, b"remote"));
        }
        batch.push(PimdirWriteOp::UpsertPlacement(card(
            collection, handle, link, "ed17", status, revision, object,
        )));
        store.write(batch).unwrap();
    }

    let conflicts = store.list_conflicts(None).unwrap();
    assert_eq!(conflicts.len(), 2, "the two clean bindings are not waiting");

    // Ordered by collection, so the calendar comes first.
    let event = &conflicts[0];
    assert_eq!(event.collection, "calendar");
    assert_eq!(event.link_id, PimdirLinkId("uid:c".into()));
    assert_eq!(event.source, PimdirSourceId("left".into()));
    assert_eq!(event.handle, PimdirHandle("event1.ics".into()));
    assert_eq!(event.conflict_revision.as_deref(), Some("r-event"));
    assert_eq!(event.conflict_object, Some(PimdirHash("evnt".into())));

    // The three bodies of a divergence, off one row: what the two sides last
    // agreed on, what this store holds, and what the remote holds.
    let card = &conflicts[1];
    assert_eq!(card.collection, "contacts");
    assert_eq!(card.link_id, PimdirLinkId("uid:a".into()));
    assert_eq!(card.base_object, Some(PimdirHash("0rig".into())));
    assert_eq!(card.object, Some(PimdirHash("ed17".into())));
    assert_eq!(card.conflict_object, Some(PimdirHash("rmte".into())));
}
