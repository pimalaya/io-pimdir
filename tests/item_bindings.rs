//! What a source's binding of one item says about it (spec §13).
//!
//! A binding is where a source's own view of an item lives: the handle it is
//! addressed by, the base the last sync agreed on, and the marker that says why
//! it might have stopped moving. The projection a client reads carries none of
//! that, so an operator asking which resource a copy came from, or why a
//! placement diverged, has nowhere else to look, which is what
//! `PimdirReader::item_bindings` answers.

use io_pimdir::{
    change::PimdirWriteOp,
    collection::PimdirCollectionId,
    hub::PimdirSourceId,
    object::PimdirHash,
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

/// One card at `handle`, all sources agreeing on the same identity.
fn card(handle: &str, status: PimdirStatus) -> PimdirPlacement {
    PimdirPlacement {
        sort_key: Default::default(),
        collection: contacts(),
        handle: PimdirHandle(handle.into()),
        link_id: Some(PimdirLinkId("card-a".into())),
        object: None,
        level: PimdirLevel::Meta,
        summary: None,
        flags: PimdirFlags::default(),
        status,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: Some("etag-1".into()),
            object: Some(PimdirHash("body".into())),
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
            PimdirWriteOp::StoreObject {
                object: io_pimdir::object::PimdirObject {
                    hash: PimdirHash("body".into()),
                    size: 3,
                },
                body: Some(b"vcf".to_vec()),
            },
            PimdirWriteOp::UpsertPlacement(card("card-a.vcf", PimdirStatus::Clean)),
        ])
        .unwrap();
    drop(store);

    let read = PimdirReader::open(dir.path()).unwrap();
    let bindings = read.item_bindings("contacts", "card-a").unwrap();

    let binding = bindings
        .get(&PimdirSourceId("carddav".into()))
        .expect("the source that wrote the item holds a binding");
    assert_eq!(binding.handle, PimdirHandle("card-a.vcf".into()));

    let base = binding.base.as_ref().expect("a synced binding has a base");
    assert_eq!(base.revision.as_deref(), Some("etag-1"));
    assert_eq!(base.object, Some(PimdirHash("body".into())));

    assert!(!binding.conflicted);
}

/// The reason to have this read at all: a minted key (spec §9) says a source
/// handed one identity over twice, and only the binding says which resource
/// each copy came from, which is the first thing an operator asks.
#[test]
fn a_minted_copy_names_the_resource_it_came_from() {
    let dir = tempdir().unwrap();
    let mut store = seed(dir.path(), "caldav");

    let mut copy = card("event-a-copy.ics", PimdirStatus::Clean);
    copy.link_id = Some(PimdirLinkId("dup:card-a#event-a-copy.ics".into()));
    store
        .write(vec![
            PimdirWriteOp::StoreObject {
                object: io_pimdir::object::PimdirObject {
                    hash: PimdirHash("body".into()),
                    size: 3,
                },
                body: Some(b"ics".to_vec()),
            },
            PimdirWriteOp::UpsertPlacement(card("event-a.ics", PimdirStatus::Clean)),
            PimdirWriteOp::UpsertPlacement(copy),
        ])
        .unwrap();
    drop(store);

    let read = PimdirReader::open(dir.path()).unwrap();
    let held = read.item_bindings("contacts", "card-a").unwrap();
    assert_eq!(
        held[&PimdirSourceId("caldav".into())].handle,
        PimdirHandle("event-a.ics".into()),
        "the item holding the bare hint keeps the resource it was bound to",
    );

    let minted = read
        .item_bindings("contacts", "dup:card-a#event-a-copy.ics")
        .unwrap();
    assert_eq!(
        minted[&PimdirSourceId("caldav".into())].handle,
        PimdirHandle("event-a-copy.ics".into()),
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
        PimdirWriteOp::StoreObject {
            object: io_pimdir::object::PimdirObject {
                hash: PimdirHash("body".into()),
                size: 3,
            },
            body: Some(b"vcf".to_vec()),
        },
        PimdirWriteOp::UpsertPlacement(card("left-handle", PimdirStatus::Clean)),
    ])
    .unwrap();
    drop(left);

    let mut right = PimdirStore::open(dir.path()).unwrap().for_source("right");
    let mut diverged = card("right-handle", PimdirStatus::Conflict);
    diverged.conflict_revision = Some("etag-remote".into());
    right
        .write(vec![PimdirWriteOp::UpsertPlacement(diverged)])
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
        bindings[&PimdirSourceId("left".into())].handle,
        PimdirHandle("left-handle".into()),
    );

    let right = &bindings[&PimdirSourceId("right".into())];
    assert_eq!(right.handle, PimdirHandle("right-handle".into()));
    assert!(right.conflicted, "the divergence belongs to one side alone");
    assert_eq!(right.conflict_revision.as_deref(), Some("etag-remote"));
    assert!(
        !bindings[&PimdirSourceId("left".into())].conflicted,
        "and the other side is untouched by it",
    );
}
