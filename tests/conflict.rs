//! An unresolved per-source content conflict survives the store.
//!
//! The sync layer's memory of "this source and its own remote diverged" lives
//! in `bindings.conflicted`, `bindings.conflict_revision` and
//! `bindings.conflict_object`. Without it the merge re-derives on every run the
//! push the remote already rejected, never converging, and a client cannot tell
//! which items need a human — so this is about the state surviving a *reopen*,
//! not just a round trip in memory.
//!
//! The body has a second requirement the revision does not: it has to outlive
//! the collector. Resolution is a person's decision, taken days after the run
//! that found the divergence, and a body swept in between leaves a revision
//! naming bytes nobody holds.

use io_pimdir::{PimdirSourceStore, PimdirStore};
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

fn contacts() -> ReplicaCollectionId {
    ReplicaCollectionId("contacts".into())
}

/// A placement of one card: where it sits, and the conflict pair its
/// binding persists.
fn card(
    collection: &str,
    handle: &str,
    link: &str,
    status: ReplicaStatus,
    conflict_revision: Option<&str>,
    conflict_object: Option<&str>,
) -> ReplicaPlacement {
    ReplicaPlacement {
        sort_key: Default::default(),
        collection: ReplicaCollectionId(collection.into()),
        handle: ReplicaHandle(handle.into()),
        link_id: Some(ReplicaLinkId(link.into())),
        object: Some(ReplicaHash("ed17".into())),
        level: ReplicaLevel::Full,
        meta: Some(ReplicaMeta(r#"{"v":1}"#.into())),
        flags: ReplicaFlags::default(),
        status,
        conflict_revision: conflict_revision.map(Into::into),
        conflict_object: conflict_object.map(|hash| ReplicaHash(hash.into())),
        base: Some(ReplicaBase {
            flags: ReplicaFlags::default(),
            revision: Some("r-base".into()),
            object: Some(ReplicaHash("0rig".into())),
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

fn seed(dir: &std::path::Path) -> PimdirSourceStore {
    let store = PimdirStore::open(dir).unwrap().for_source("left");
    store.ensure_collection("contacts", "text/vcard").unwrap();
    store
}

/// A placement plus the bodies it points at, as one batch.
///
/// They must ride in the **same** batch: an object no placement references has
/// refcount 0 and is swept at the end of its own batch, so storing them
/// separately would leave the placement's foreign key dangling.
fn card_batch(
    status: ReplicaStatus,
    conflict_revision: Option<&str>,
    conflict_object: Option<&str>,
) -> Vec<ReplicaWriteOp> {
    let mut batch = vec![store_object("0rig", b"old"), store_object("ed17", b"new")];

    if let Some(hash) = conflict_object {
        batch.push(store_object(hash, b"remote"));
    }

    batch.push(ReplicaWriteOp::UpsertPlacement(card(
        "contacts",
        "card1.vcf",
        "uid:a",
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
            ReplicaStatus::Conflict,
            Some("r-remote"),
            Some("rmte"),
        ))
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
    assert_eq!(
        loaded.placements[0].conflict_object,
        Some(ReplicaHash("rmte".into())),
        "and the body at that revision is what it merges, with no remote to ask"
    );
}

#[test]
fn resolving_the_conflict_clears_it_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seed(dir.path());

    store
        .write(card_batch(
            ReplicaStatus::Conflict,
            Some("r-remote"),
            Some("rmte"),
        ))
        .unwrap();
    // The consumer resolves with an ordinary edit — no dedicated call.
    store
        .write(card_batch(ReplicaStatus::Dirty, None, None))
        .unwrap();
    drop(store);

    let store = PimdirStore::open(dir.path()).unwrap().for_source("left");
    let loaded = store.load(&contacts(), &ReplicaLoadScope::All).unwrap();
    assert_ne!(loaded.placements[0].status, ReplicaStatus::Conflict);
    assert_eq!(
        loaded.placements[0].conflict_revision, None,
        "a resolved binding must not carry a stale revision forward"
    );
    assert_eq!(
        loaded.placements[0].conflict_object, None,
        "nor the body that revision named, which the remote may have replaced"
    );
}

#[test]
fn a_store_from_an_earlier_draft_of_v1_is_reconciled_on_open() {
    // The draft allowance (spec §6): the three columns were folded into version
    // 1 after it was published, so a store written by an earlier draft is
    // stamped `user_version = 1` yet lacks them. It must be healed on open, not
    // left to fail on the next query.
    let dir = tempfile::tempdir().unwrap();
    let store = PimdirStore::open(dir.path()).unwrap();
    drop(store);

    // Rewind the store to the earlier draft's shape. The two indexes go
    // first: SQLite refuses to drop a column an index names, and a draft
    // without the columns could not have carried them either.
    let db = dir.path().join("pimdir.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("DROP INDEX bindings_conflicted")
        .unwrap();
    conn.execute_batch("DROP INDEX bindings_by_conflict_object")
        .unwrap();
    conn.execute_batch("ALTER TABLE bindings DROP COLUMN conflicted")
        .unwrap();
    conn.execute_batch("ALTER TABLE bindings DROP COLUMN conflict_revision")
        .unwrap();
    conn.execute_batch("ALTER TABLE bindings DROP COLUMN conflict_object")
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
        .write(card_batch(
            ReplicaStatus::Conflict,
            Some("r-remote"),
            Some("rmte"),
        ))
        .unwrap();

    let loaded = store.load(&contacts(), &ReplicaLoadScope::All).unwrap();
    assert_eq!(loaded.placements[0].status, ReplicaStatus::Conflict);
    assert_eq!(
        loaded.placements[0].conflict_revision.as_deref(),
        Some("r-remote")
    );
    assert_eq!(
        loaded.placements[0].conflict_object,
        Some(ReplicaHash("rmte".into())),
        "the column is not just back, it holds what the sync wrote through it"
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
            ReplicaStatus::Conflict,
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

    let loaded = store.load(&contacts(), &ReplicaLoadScope::All).unwrap();
    assert_eq!(
        loaded.placements[0].conflict_object,
        Some(ReplicaHash("rmte".into()))
    );
}

/// The other half: the pin is a pin, not a leak.
///
/// A resolution is an ordinary edit, so it clears the binding's conflict and
/// with it the only reference to the remote body. The next collection takes it
/// like any other unreferenced object.
#[test]
fn resolving_releases_the_pin_and_the_next_collection_takes_the_body() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seed(dir.path());

    store
        .write(card_batch(
            ReplicaStatus::Conflict,
            Some("r-remote"),
            Some("rmte"),
        ))
        .unwrap();
    store
        .write(card_batch(ReplicaStatus::Dirty, None, None))
        .unwrap();

    let collected = store.collect_garbage().unwrap();
    assert_eq!((collected.objects, collected.blobs), (1, 1));
    assert!(
        !blob_exists(dir.path(), "rmte"),
        "a resolved conflict holds nothing"
    );
    assert!(blob_exists(dir.path(), "ed17"), "the item's own body stays");
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
            ReplicaStatus::Conflict,
            Some("r-remote"),
            Some("rmte"),
        ),
        (
            "contacts",
            "card2.vcf",
            "uid:b",
            ReplicaStatus::Clean,
            None,
            None,
        ),
        (
            "calendar",
            "event1.ics",
            "uid:c",
            ReplicaStatus::Conflict,
            Some("r-event"),
            Some("evnt"),
        ),
        (
            "calendar",
            "event2.ics",
            "uid:d",
            ReplicaStatus::Clean,
            None,
            None,
        ),
    ] {
        let mut batch = vec![store_object("0rig", b"old"), store_object("ed17", b"new")];
        if let Some(hash) = object {
            batch.push(store_object(hash, b"remote"));
        }
        batch.push(ReplicaWriteOp::UpsertPlacement(card(
            collection, handle, link, status, revision, object,
        )));
        store.write(batch).unwrap();
    }

    let conflicts = store.list_conflicts(None).unwrap();
    assert_eq!(conflicts.len(), 2, "the two clean bindings are not waiting");

    // Ordered by collection, so the calendar comes first.
    let event = &conflicts[0];
    assert_eq!(event.collection, "calendar");
    assert_eq!(event.link_id, ReplicaLinkId("uid:c".into()));
    assert_eq!(event.source, ReplicaSourceId("left".into()));
    assert_eq!(event.handle, ReplicaHandle("event1.ics".into()));
    assert_eq!(event.conflict_revision.as_deref(), Some("r-event"));
    assert_eq!(event.conflict_object, Some(ReplicaHash("evnt".into())));

    // The three bodies of a divergence, off one row: what the two sides last
    // agreed on, what this store holds, and what the remote holds.
    let card = &conflicts[1];
    assert_eq!(card.collection, "contacts");
    assert_eq!(card.link_id, ReplicaLinkId("uid:a".into()));
    assert_eq!(card.base_object, Some(ReplicaHash("0rig".into())));
    assert_eq!(card.object, Some(ReplicaHash("ed17".into())));
    assert_eq!(card.conflict_object, Some(ReplicaHash("rmte".into())));
}
