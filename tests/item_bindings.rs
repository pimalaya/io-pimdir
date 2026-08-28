//! What a source's binding of one item says about it (spec §13).
//!
//! A binding is where a source's own view of an item lives: the handle it is
//! addressed by, the base the last sync agreed on, and the marker that says why
//! it might have stopped moving. The projection a client reads carries none of
//! that, so an operator asking which resource a copy came from, or why a
//! placement diverged, has nowhere else to look, which is what
//! `PimdirReader::item_bindings` answers.

use io_pimdir::{PimdirReader, PimdirSourceStore, PimdirStore};
use io_replica::{
    change::ReplicaWriteOp,
    client::ReplicaStorage,
    collection::ReplicaCollectionId,
    hub::ReplicaSourceId,
    object::ReplicaHash,
    placement::{
        ReplicaBase, ReplicaFlags, ReplicaHandle, ReplicaLevel, ReplicaLinkId, ReplicaMeta,
        ReplicaPlacement, ReplicaStatus,
    },
};
use tempfile::tempdir;

fn contacts() -> ReplicaCollectionId {
    ReplicaCollectionId("contacts".into())
}

/// One card at `handle`, all sources agreeing on the same identity.
fn card(handle: &str, status: ReplicaStatus) -> ReplicaPlacement {
    ReplicaPlacement {
        sort_key: Default::default(),
        collection: contacts(),
        handle: ReplicaHandle(handle.into()),
        link_id: Some(ReplicaLinkId("card-a".into())),
        object: None,
        level: ReplicaLevel::Meta,
        meta: Some(ReplicaMeta(r#"{"v":1}"#.into())),
        flags: ReplicaFlags::default(),
        status,
        conflict_revision: None,
        conflict_object: None,
        base: Some(ReplicaBase {
            flags: ReplicaFlags::default(),
            revision: Some("etag-1".into()),
            object: Some(ReplicaHash("body".into())),
        }),
        origin: None,
    }
}

fn seed(dir: &std::path::Path, source: &str) -> PimdirSourceStore {
    let store = PimdirStore::open(dir).unwrap().for_source(source);
    store.ensure_collection("contacts", "text/vcard").unwrap();
    store
}

#[test]
fn a_binding_names_the_handle_and_the_base_its_source_agreed_on() {
    let dir = tempdir().unwrap();
    let mut store = seed(dir.path(), "carddav");
    store
        .write(vec![
            ReplicaWriteOp::StoreObject {
                object: io_replica::object::ReplicaObject {
                    hash: ReplicaHash("body".into()),
                    size: 3,
                },
                body: Some(b"vcf".to_vec()),
            },
            ReplicaWriteOp::UpsertPlacement(card("card-a.vcf", ReplicaStatus::Clean)),
        ])
        .unwrap();
    drop(store);

    let read = PimdirReader::open(dir.path()).unwrap();
    let bindings = read.item_bindings("contacts", "card-a").unwrap();

    let binding = bindings
        .get(&ReplicaSourceId("carddav".into()))
        .expect("the source that wrote the item holds a binding");
    assert_eq!(binding.handle, ReplicaHandle("card-a.vcf".into()));

    let base = binding.base.as_ref().expect("a synced binding has a base");
    assert_eq!(base.revision.as_deref(), Some("etag-1"));
    assert_eq!(base.object, Some(ReplicaHash("body".into())));

    assert!(!binding.conflicted);
}

/// The reason to have this read at all: a minted key (spec §9) says a source
/// handed one identity over twice, and only the binding says which resource
/// each copy came from, which is the first thing an operator asks.
#[test]
fn a_minted_copy_names_the_resource_it_came_from() {
    let dir = tempdir().unwrap();
    let mut store = seed(dir.path(), "caldav");

    let mut copy = card("event-a-copy.ics", ReplicaStatus::Clean);
    copy.link_id = Some(ReplicaLinkId("dup:card-a#event-a-copy.ics".into()));
    store
        .write(vec![
            ReplicaWriteOp::StoreObject {
                object: io_replica::object::ReplicaObject {
                    hash: ReplicaHash("body".into()),
                    size: 3,
                },
                body: Some(b"ics".to_vec()),
            },
            ReplicaWriteOp::UpsertPlacement(card("event-a.ics", ReplicaStatus::Clean)),
            ReplicaWriteOp::UpsertPlacement(copy),
        ])
        .unwrap();
    drop(store);

    let read = PimdirReader::open(dir.path()).unwrap();
    let held = read.item_bindings("contacts", "card-a").unwrap();
    assert_eq!(
        held[&ReplicaSourceId("caldav".into())].handle,
        ReplicaHandle("event-a.ics".into()),
        "the item holding the bare hint keeps the resource it was bound to",
    );

    let minted = read
        .item_bindings("contacts", "dup:card-a#event-a-copy.ics")
        .unwrap();
    assert_eq!(
        minted[&ReplicaSourceId("caldav".into())].handle,
        ReplicaHandle("event-a-copy.ics".into()),
        "and the minted copy has a binding of its own, naming the other resource",
    );
}

/// Two sources hold one item, and each has its own view of it: a read that
/// folded them into one would say nothing about which side diverged.
#[test]
fn every_source_holding_an_item_reports_its_own_binding() {
    let dir = tempdir().unwrap();

    let mut left = seed(dir.path(), "left");
    left.write(vec![
        ReplicaWriteOp::StoreObject {
            object: io_replica::object::ReplicaObject {
                hash: ReplicaHash("body".into()),
                size: 3,
            },
            body: Some(b"vcf".to_vec()),
        },
        ReplicaWriteOp::UpsertPlacement(card("left-handle", ReplicaStatus::Clean)),
    ])
    .unwrap();
    drop(left);

    let mut right = PimdirStore::open(dir.path()).unwrap().for_source("right");
    let mut diverged = card("right-handle", ReplicaStatus::Conflict);
    diverged.conflict_revision = Some("etag-remote".into());
    right
        .write(vec![ReplicaWriteOp::UpsertPlacement(diverged)])
        .unwrap();
    drop(right);

    let read = PimdirReader::open(dir.path()).unwrap();
    let bindings = read.item_bindings("contacts", "card-a").unwrap();

    assert_eq!(
        bindings.len(),
        2,
        "one binding per source, not one per item"
    );
    assert_eq!(
        bindings[&ReplicaSourceId("left".into())].handle,
        ReplicaHandle("left-handle".into()),
    );

    let right = &bindings[&ReplicaSourceId("right".into())];
    assert_eq!(right.handle, ReplicaHandle("right-handle".into()));
    assert!(right.conflicted, "the divergence belongs to one side alone");
    assert_eq!(right.conflict_revision.as_deref(), Some("etag-remote"));
    assert!(
        !bindings[&ReplicaSourceId("left".into())].conflicted,
        "and the other side is untouched by it",
    );
}
