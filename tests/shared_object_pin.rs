//! The agreement point names a body and pins none (spec §5, §13).
//!
//! `bindings.shared_object` is the one pointer at an object the refcount
//! deliberately does not count. The format says so twice: §5 calls it
//! "the one exception and it is deliberate", and §13 repeats that it
//! "names an object and MUST NOT be counted in the refcount above",
//! because it is only ever compared for equality and never read as bytes,
//! and a content hash compares the same after the body it named has been
//! swept. Counting it would pin every body a source ever agreed with for
//! the life of the binding.
//!
//! The asymmetry with `conflict_object` beside it is what invites a
//! future reader to "fix" it, so it is nailed down here from both ends: a
//! body no column but this one names is collectable, and the column still
//! reads back afterwards, which is the whole argument for not pinning.

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

/// The decision, from both ends: a body only the agreement point names is
/// collectable, and the column survives the collection that took it.
///
/// A source that has folded once and then stops writing keeps the
/// agreement point it settled on while the hub moves under it, which is
/// the only way a `shared_object` can lag behind every other pointer.
#[test]
fn a_body_named_only_by_an_agreement_point_is_collectable() {
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

    // Nothing counted it: the incremental write path and the
    // recomputation over the five pointer columns both put "one" at zero,
    // so the drift read that compares them is empty.
    assert!(
        left.refcount_drift().unwrap().is_empty(),
        "counting the agreement point would put the write path and the \
         five-column recomputation at odds about this body"
    );
    assert!(
        blob_of(dir.path(), &one).is_file(),
        "still there, since no write reclaims (spec §5)"
    );

    let collected = left.collect_garbage().unwrap();
    assert_eq!(
        (collected.objects, collected.blobs),
        (1, 1),
        "the agreement point held no pin, so the body it names is taken"
    );
    assert!(!blob_of(dir.path(), &one).is_file(), "the body is gone");
    assert!(
        blob_of(dir.path(), &two).is_file(),
        "the shared body is still referenced and untouched"
    );

    // The whole argument for not pinning: a content hash compares the
    // same after the body it named has been swept, so the column still
    // does its job and the store still opens and reads.
    drop(left);
    drop(right);
    let reopened = store(dir.path(), "right");
    assert_eq!(agreement(&reopened, "right"), Some(one));
    assert!(reopened.refcount_drift().unwrap().is_empty());
}

/// The pin beside it, so the two are asserted against each other rather
/// than each in isolation.
///
/// `conflict_object` and `shared_object` are two hashes on one row,
/// written by one statement. Only one of them is a reference, and a test
/// that checked either alone would pass on an implementation that counted
/// both or neither.
#[test]
fn the_conflict_body_is_pinned_where_the_agreement_point_is_not() {
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
