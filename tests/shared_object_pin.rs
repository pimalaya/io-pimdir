//! The agreement point pins the body it names (STORAGE §5).
//!
//! `bindings.shared_object` names the body a binding last agreed with the
//! shared item, the common ancestor an engine reads to merge a source's
//! body with the item's (SYNC §9). It is counted as `bindings.base_object`
//! is, so it outlives the item's own body until the binding reconciles
//! against what replaced it: one body per binding, released when the
//! binding agrees with another. An earlier draft left it unpinned, and a
//! collector could sweep the very ancestor a later merge needed.

use std::path::Path;

use io_pimdir::client::{PimdirSourceStore, PimdirStore};
use io_pimdir::{
    change::PimdirWriteOp,
    collection::PimdirCollectionId,
    hub::PimdirSourceId,
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirSortKey, PimdirStatus,
    },
};

const CONTACTS: &str = "contacts";
const LINK: &str = "uid:a";

fn store(dir: &Path, source: &str) -> PimdirSourceStore {
    let store = PimdirStore::open(dir).unwrap().for_source(source);
    store.ensure_collection(CONTACTS, "text/vcard").unwrap();
    store
}

fn body(store: &PimdirSourceStore, bytes: &[u8]) -> PimdirWriteOp {
    PimdirWriteOp::StoreObject {
        object: PimdirObject {
            hash: store.hash(bytes),
            size: bytes.len(),
        },
        body: Some(bytes.to_vec()),
    }
}

/// One source's placement of the card, with the body it carries and the
/// base it agreed with its own remote.
fn card(
    store: &PimdirSourceStore,
    handle: &str,
    object: &[u8],
    base: Option<&[u8]>,
) -> PimdirWriteOp {
    PimdirWriteOp::UpsertPlacement(PimdirPlacement {
        collection: PimdirCollectionId(CONTACTS.into()),
        handle: PimdirHandle(handle.into()),
        link_id: Some(PimdirLinkId(LINK.into())),
        object: Some(store.hash(object)),
        level: PimdirLevel::Full,
        summary: None,
        sort_key: PimdirSortKey("k".into()),
        flags: PimdirFlags::default(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: base.map(|bytes| PimdirBase {
            flags: PimdirFlags::default(),
            revision: Some("r".into()),
            object: Some(store.hash(bytes)),
        }),
        origin: None,
    })
}

/// The agreement point one source's binding of the card currently holds.
fn agreement(store: &PimdirSourceStore, source: &str) -> Option<PimdirHash> {
    store
        .item_bindings(CONTACTS, LINK)
        .unwrap()
        .into_iter()
        .find(|(id, _)| id == &PimdirSourceId(source.into()))
        .map(|(_, binding)| binding.shared_object)
        .unwrap()
}

fn blob_of(dir: &Path, hash: &PimdirHash) -> std::path::PathBuf {
    dir.join("objects")
        .join(&hash.0[0..2])
        .join(&hash.0[2..4])
        .join(&hash.0)
}

/// The rule, from both ends: a body only the agreement point names
/// survives the collector, and goes once the binding agrees with another.
///
/// A source that has folded once and then stops writing keeps the
/// agreement point it settled on while the hub moves under it, which is
/// the only way a `shared_object` can lag behind every other pointer.
#[test]
fn a_body_named_only_by_an_agreement_point_is_pinned_until_it_moves() {
    let dir = tempfile::tempdir().unwrap();
    let mut left = store(dir.path(), "left");
    let mut right = store(dir.path(), "right");

    // Left files the card with body "one" and agrees with it.
    left.write(vec![
        body(&left, b"one"),
        card(&left, "left-1.vcf", b"one", Some(b"one")),
    ])
    .unwrap();

    // Right folds the same body in, carrying no base of its own: its
    // binding therefore names the body through `shared_object` alone.
    right
        .write(vec![
            body(&right, b"one"),
            card(&right, "right-1.vcf", b"one", None),
        ])
        .unwrap();

    // Left edits to "two" and pushes, moving the hub's body and its own
    // base with it. Right never writes again, so its agreement point is
    // left naming "one".
    left.write(vec![
        body(&left, b"two"),
        card(&left, "left-1.vcf", b"two", Some(b"two")),
    ])
    .unwrap();

    let one = left.hash(b"one");
    let two = left.hash(b"two");
    assert_eq!(agreement(&right, "right"), Some(one.clone()));
    assert_eq!(agreement(&left, "left"), Some(two.clone()));

    // The write path counted it as the six-column recomputation does.
    assert!(left.refcount_drift().unwrap().is_empty());

    let collected = left.collect_garbage().unwrap();
    assert_eq!(
        (collected.objects, collected.blobs),
        (0, 0),
        "the agreement point pins the body it names"
    );
    assert_eq!(
        std::fs::read(blob_of(dir.path(), &one)).unwrap(),
        b"one",
        "the common ancestor a merge reads is readable as bytes"
    );

    // Right agrees with "two": its pin moves, and "one" falls.
    right
        .write(vec![card(&right, "right-1.vcf", b"two", None)])
        .unwrap();
    assert_eq!(agreement(&right, "right"), Some(two.clone()));
    assert!(right.refcount_drift().unwrap().is_empty());
    let collected = left.collect_garbage().unwrap();
    assert_eq!((collected.objects, collected.blobs), (1, 1));
    assert!(!blob_of(dir.path(), &one).is_file(), "released, then taken");
    assert!(blob_of(dir.path(), &two).is_file());
}

/// The pin beside it: `conflict_object` and `shared_object` are two
/// hashes on one row, written by one statement, both references.
#[test]
fn the_conflict_body_is_pinned_as_the_agreement_point_is() {
    let dir = tempfile::tempdir().unwrap();
    let mut left = store(dir.path(), "left");

    // A binding stuck on a divergence: its remote holds "remote", which
    // it has fetched but not resolved.
    left.write(vec![
        body(&left, b"one"),
        body(&left, b"remote"),
        PimdirWriteOp::UpsertPlacement(PimdirPlacement {
            collection: PimdirCollectionId(CONTACTS.into()),
            handle: PimdirHandle("left-1.vcf".into()),
            link_id: Some(PimdirLinkId(LINK.into())),
            object: Some(left.hash(b"one")),
            level: PimdirLevel::Full,
            summary: None,
            sort_key: PimdirSortKey("k".into()),
            flags: PimdirFlags::default(),
            status: PimdirStatus::Conflict,
            conflict_revision: Some("r-remote".into()),
            conflict_object: Some(left.hash(b"remote")),
            base: Some(PimdirBase {
                flags: PimdirFlags::default(),
                revision: Some("r".into()),
                object: Some(left.hash(b"one")),
            }),
            origin: None,
        }),
    ])
    .unwrap();

    let remote = left.hash(b"remote");
    assert!(left.refcount_drift().unwrap().is_empty());

    // The diverging body is a reference, so the collector leaves it: a
    // person resolves the conflict days after the run that found it, and
    // an unpinned body does not survive that interval.
    let collected = left.collect_garbage().unwrap();
    assert_eq!((collected.objects, collected.blobs), (0, 0));
    assert_eq!(
        std::fs::read(blob_of(dir.path(), &remote)).unwrap(),
        b"remote",
        "the diverging body is readable as bytes, not merely present"
    );
}
