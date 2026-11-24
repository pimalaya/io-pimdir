//! A relocation names the member it made (pimdir SYNC §4, STORAGE §14):
//! the write recording a relocation that stated its target handle lands
//! the destination's create on it, pending or delivered, and one whose
//! handle the source already binds there records the create delivered.

#![cfg(feature = "client")]

use std::path::Path;

use io_pimdir::{
    change::PimdirWriteOp,
    client::{PimdirSourceStore, PimdirStore, producer::PimdirProducer},
    codec::PimdirAction,
    collection::PimdirCollectionId,
    load::{PimdirLoadSelection, PimdirLoaded},
    object::PimdirHash,
    placement::{PimdirFlags, PimdirHandle, PimdirLinkId, PimdirPlacement},
};

/// A message `id` dated in August 2026.
fn mail(id: &str) -> Vec<u8> {
    format!(
        "Message-ID: <{id}>\r\nFrom: Alice <alice@example.org>\r\nSubject: {id}\r\n\
         Date: Sat, 1 Aug 2026 10:00:00 +0000\r\n\r\nbody of {id}\r\n"
    )
    .into_bytes()
}

fn add(hash: PimdirHash) -> PimdirAction {
    PimdirAction::Add {
        link_id: None,
        flags: PimdirFlags::from_iter(["\\Seen"]),
        object: Some(hash),
        supersedes: None,
    }
}

/// A store whose INBOX holds the messages `ids`, each a create queued and
/// applied, pending for the source `local`.
fn populated(dir: &Path, ids: &[&str]) -> PimdirSourceStore {
    let mut store = PimdirStore::open(dir).unwrap().for_source("local");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    let mut producer = PimdirProducer::open(dir, "test").unwrap();
    for id in ids {
        producer.enqueue_with_body("INBOX", &mail(id), add).unwrap();
    }
    drop(producer);
    assert_eq!(store.drain().unwrap().applied, ids.len());
    store
}

fn inbox() -> PimdirCollectionId {
    PimdirCollectionId("INBOX".into())
}

fn provisional(link: &str) -> PimdirHandle {
    PimdirLinkId(link.into()).provisional()
}

fn land(link: &str, landed: &str) -> PimdirWriteOp {
    PimdirWriteOp::LandPlacement {
        collection: inbox(),
        handle: provisional(link),
        landed: PimdirHandle::from(landed),
        flags: PimdirFlags::from_iter(["\\Seen"]),
        revision: None,
    }
}

fn loaded(store: &mut PimdirSourceStore) -> PimdirLoaded {
    store.load(&inbox(), &PimdirLoadSelection::All).unwrap()
}

fn placement<'a>(loaded: &'a PimdirLoaded, link: &str) -> &'a PimdirPlacement {
    loaded
        .placements
        .iter()
        .find(|p| p.link_id.as_ref().is_some_and(|l| l.0 == link))
        .unwrap_or_else(|| panic!("{link} in {:?}", loaded.placements))
}

/// A pending create lands on the stated handle: the provisional handle
/// is gone, the binding is under the stated one, and its base holds the
/// flags and the body staged on the create, so it owes nothing.
#[test]
fn a_relocation_stating_its_handle_lands_a_pending_create() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = populated(dir.path(), &["a@x"]);

    store.write(vec![land("a@x", "7")]).unwrap();

    let loaded = loaded(&mut store);
    let landed = placement(&loaded, "a@x");
    assert_eq!(landed.handle.as_str(), "7");
    let base = landed.base.as_ref().expect("a base");
    assert_eq!(base.flags, PimdirFlags::from_iter(["\\Seen"]));
    assert_eq!(base.object, landed.object);
    assert!(base.object.is_some(), "the staged body is the base");
    assert!(!landed.delivered());
    assert_eq!(loaded.placements.len(), 1, "{:?}", loaded.placements);
}

/// A create recorded delivered first, as an owner staging while a push is
/// in flight may, lands all the same.
#[test]
fn a_relocation_stating_its_handle_lands_a_delivered_create() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = populated(dir.path(), &["a@x"]);
    store
        .write(vec![PimdirWriteOp::DeliverPlacement {
            collection: inbox(),
            handle: provisional("a@x"),
        }])
        .unwrap();
    assert!(placement(&loaded(&mut store), "a@x").delivered());

    store.write(vec![land("a@x", "7")]).unwrap();

    let loaded = loaded(&mut store);
    let landed = placement(&loaded, "a@x");
    assert_eq!(landed.handle.as_str(), "7");
    assert!(landed.base.as_ref().is_some_and(|b| b.object.is_some()));
}

/// A flag change staged on a delivered create leaves its base naming
/// nothing (SYNC §2), so a relocation stating its handle still lands it
/// (pimdir vector 221). Given the item's body as its base, the create was
/// no delivered one any more and the landing passed it by.
#[test]
fn a_delivered_create_flagged_since_lands_all_the_same() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = populated(dir.path(), &["a@x"]);
    store
        .write(vec![PimdirWriteOp::DeliverPlacement {
            collection: inbox(),
            handle: provisional("a@x"),
        }])
        .unwrap();
    let seq = store.seq_for_link("INBOX", "a@x").unwrap().unwrap();
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let flagged = PimdirAction::SetFlags {
        seq,
        flags: PimdirFlags::from_iter(["\\Flagged", "\\Seen"]),
        base: None,
    };
    producer.enqueue("INBOX", &flagged, None).unwrap();
    drop(producer);
    assert_eq!(store.drain().unwrap().applied, 1);
    let flagged = loaded(&mut store);
    let delivered = placement(&flagged, "a@x");
    assert!(delivered.delivered());
    assert_eq!(delivered.base.as_ref().unwrap().object, None);

    store.write(vec![land("a@x", "7")]).unwrap();

    let loaded = loaded(&mut store);
    assert_eq!(placement(&loaded, "a@x").handle.as_str(), "7");
}

/// A stated handle the source already binds in the collection is no
/// member to move the create to: the create is recorded delivered, for a
/// listing to land, and the binding holding the handle is left as it is.
#[test]
fn a_landing_on_a_bound_handle_delivers_instead() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = populated(dir.path(), &["a@x", "b@x"]);
    store.write(vec![land("b@x", "7")]).unwrap();

    store.write(vec![land("a@x", "7")]).unwrap();

    let loaded = loaded(&mut store);
    let a = placement(&loaded, "a@x");
    assert_eq!(a.handle, provisional("a@x"));
    assert!(a.delivered(), "{a:?}");
    assert_eq!(placement(&loaded, "b@x").handle.as_str(), "7");
}
