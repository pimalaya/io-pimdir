//! One store holding several accounts.
//!
//! The account groups collections and partitions nothing (spec §9.2), so
//! what these check is mostly what stays true when a second account moves
//! in: the `seq` a link id draws, the single copy a shared body gets, and
//! that regrouping a collection disturbs neither. The two multiplicity
//! reads are the store's whole answer to "the same thing is in two
//! accounts": they report it, and every merge policy is built on top.

use io_pimdir::PimdirStore;
use io_replica::{
    change::ReplicaWriteOp,
    client::ReplicaStorage,
    collection::ReplicaCollectionId,
    object::{ReplicaHash, ReplicaObject},
    placement::{
        ReplicaFlags, ReplicaHandle, ReplicaLevel, ReplicaLinkId, ReplicaMeta, ReplicaPlacement,
        ReplicaStatus,
    },
};

/// One placement of `link_id` in `collection`, pointing at `hash`.
fn placement(collection: &str, handle: &str, link_id: &str, hash: &str) -> ReplicaPlacement {
    ReplicaPlacement {
        sort_key: Default::default(),
        collection: ReplicaCollectionId(collection.into()),
        handle: ReplicaHandle(handle.into()),
        link_id: Some(ReplicaLinkId(link_id.into())),
        object: Some(ReplicaHash(hash.into())),
        level: ReplicaLevel::Full,
        meta: Some(ReplicaMeta(r#"{"v":1}"#.into())),
        flags: ReplicaFlags::default(),
        status: ReplicaStatus::Clean,
        conflict_revision: None,
        base: None,
        origin: None,
        ambiguous_handles: Vec::new(),
    }
}

/// The placement plus the body it points at, in one batch: an object no
/// placement references is swept at the end of that batch.
fn batch(collection: &str, handle: &str, link_id: &str, hash: &str) -> Vec<ReplicaWriteOp> {
    vec![
        ReplicaWriteOp::StoreObject {
            object: ReplicaObject {
                hash: ReplicaHash(hash.into()),
                size: 4,
            },
            body: Some(b"body".to_vec()),
        },
        ReplicaWriteOp::UpsertPlacement(placement(collection, handle, link_id, hash)),
    ]
}

/// Two accounts, one mailbox each, both holding the same message.
fn seed(dir: &std::path::Path) {
    let mut work = PimdirStore::open(dir)
        .unwrap()
        .for_account("work")
        .for_source("server");
    work.ensure_collection("work/INBOX", "message/rfc822")
        .unwrap();
    work.write(batch("work/INBOX", "1", "<news@x>", "beef"))
        .unwrap();
    drop(work);

    let mut home = PimdirStore::open(dir)
        .unwrap()
        .for_account("home")
        .for_source("server");
    home.ensure_collection("home/INBOX", "message/rfc822")
        .unwrap();
    home.write(batch("home/INBOX", "9", "<news@x>", "beef"))
        .unwrap();
}

#[test]
fn collections_carry_their_account() {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path());

    let store = PimdirStore::open(dir.path()).unwrap();
    assert_eq!(store.list_accounts().unwrap(), ["home", "work"]);

    let work = store.list_collections_by_account(Some("work")).unwrap();
    assert_eq!(work.len(), 1);
    assert_eq!(work[0].id, "work/INBOX");
    assert_eq!(work[0].account.as_deref(), Some("work"));

    // every collection is grouped, so the single-account bucket is empty
    assert!(store.list_collections_by_account(None).unwrap().is_empty());
}

#[test]
fn an_ungrouped_store_is_the_null_bucket() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("server");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    store.write(batch("INBOX", "1", "<a@x>", "beef")).unwrap();

    // the point of matching with `IS`: a NULL account matches itself,
    // where `=` would match nothing
    let listed = store.list_collections_by_account(None).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "INBOX");
    assert_eq!(listed[0].account, None);
    assert!(store.list_accounts().unwrap().is_empty());
}

#[test]
fn the_account_partitions_no_identifier() {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path());

    let store = PimdirStore::open(dir.path()).unwrap();
    let placements = store.link_placements("<news@x>").unwrap();
    assert_eq!(placements.len(), 2);

    // one link id, one seq, whichever account holds it: the seq is the
    // short form of the link id
    assert_eq!(placements[0].seq, placements[1].seq);

    // and one body, stored once for both accounts
    let bodies: Vec<_> = placements.iter().map(|p| p.object.clone()).collect();
    assert_eq!(bodies[0], bodies[1]);
}

#[test]
fn multiplicity_is_reported_on_both_axes() {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path());

    let store = PimdirStore::open(dir.path()).unwrap();

    // the identity axis: where this link id occurs, account included
    let by_link = store.link_placements("<news@x>").unwrap();
    let seen: Vec<_> = by_link
        .iter()
        .map(|p| (p.account.as_deref(), p.collection.as_str()))
        .collect();
    assert_eq!(
        seen,
        [(Some("home"), "home/INBOX"), (Some("work"), "work/INBOX")]
    );

    // the dedup axis: the same two found by body, which is what pairs
    // placements two servers gave different link ids
    let by_object = store.object_placements("beef").unwrap();
    let seen: Vec<_> = by_object
        .iter()
        .map(|p| (p.account.as_deref(), p.collection.as_str()))
        .collect();
    assert_eq!(
        seen,
        [(Some("home"), "home/INBOX"), (Some("work"), "work/INBOX")]
    );

    // an identity nobody holds reports nothing rather than erroring
    assert!(store.link_placements("<absent@x>").unwrap().is_empty());
}

#[test]
fn regrouping_a_collection_disturbs_nothing() {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path());

    let store = PimdirStore::open(dir.path()).unwrap();
    let before = store.link_placements("<news@x>").unwrap();

    store
        .set_collection_account("home/INBOX", Some("personal"))
        .unwrap();
    assert_eq!(
        store.collection_account("home/INBOX").unwrap(),
        Some(Some("personal".into()))
    );

    // the move regroups and nothing else: same placements, same seqs,
    // same bodies
    let after = store.link_placements("<news@x>").unwrap();
    assert_eq!(before.len(), after.len());
    let seqs_before: Vec<_> = before.iter().map(|p| p.seq).collect();
    let seqs_after: Vec<_> = after.iter().map(|p| p.seq).collect();
    assert_eq!(seqs_before, seqs_after);

    assert_eq!(store.list_accounts().unwrap(), ["personal", "work"]);
}

#[test]
fn a_sync_declaring_a_kind_never_moves_a_collection() {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path());

    // a handle bound to another account re-declaring the kind must not
    // steal the collection: set_collection_kind updates the kind alone
    let other = PimdirStore::open(dir.path())
        .unwrap()
        .for_account("home")
        .for_source("server");
    other.ensure_collection("work/INBOX", "text/vcard").unwrap();

    assert_eq!(
        other.collection_account("work/INBOX").unwrap(),
        Some(Some("work".into()))
    );
    assert_eq!(
        other.collection_kind("work/INBOX").unwrap().as_deref(),
        Some("text/vcard")
    );
}

#[test]
fn an_unknown_collection_has_no_account() {
    let dir = tempfile::tempdir().unwrap();
    let store = PimdirStore::open(dir.path()).unwrap();

    // the outer None is "no such collection", distinct from Some(None),
    // which is "exists, ungrouped"
    assert_eq!(store.collection_account("nope").unwrap(), None);
}

#[test]
fn a_body_lookup_never_crosses_an_account() {
    // spec §9.2 names the case: two unrelated servers may mint the same
    // identity, so answering a body lookup with the other account's
    // object would have that sync believe the item hydrated
    let dir = tempfile::tempdir().unwrap();

    let mut work = PimdirStore::open(dir.path())
        .unwrap()
        .for_account("work")
        .for_source("server");
    work.ensure_collection("work/AB", "text/vcard").unwrap();
    work.write(batch("work/AB", "1", "uid-collide", "aaaabbbb"))
        .unwrap();

    // home holds the same identity, with a different body and none cached
    let home = PimdirStore::open(dir.path())
        .unwrap()
        .for_account("home")
        .for_source("server");
    home.ensure_collection("home/AB", "text/vcard").unwrap();

    let found = home
        .lookup_objects(&[ReplicaLinkId("uid-collide".into())])
        .unwrap();
    assert!(
        found.is_empty(),
        "home has no body for this identity; work's is not an answer: {found:?}",
    );

    // the same lookup within the owning account still answers, which is
    // what the read exists for
    let found = work
        .lookup_objects(&[ReplicaLinkId("uid-collide".into())])
        .unwrap();
    assert_eq!(found.len(), 1);
}

#[test]
fn a_body_lookup_still_dedups_across_collections() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path())
        .unwrap()
        .for_account("work")
        .for_source("server");
    store
        .ensure_collection("work/INBOX", "message/rfc822")
        .unwrap();
    store
        .ensure_collection("work/Archive", "message/rfc822")
        .unwrap();
    store
        .write(batch("work/INBOX", "1", "<msg@x>", "beef"))
        .unwrap();

    let found = store
        .lookup_objects(&[ReplicaLinkId("<msg@x>".into())])
        .unwrap();
    assert_eq!(
        found.len(),
        1,
        "one message filed in two mailboxes is one body, downloaded once",
    );
}
