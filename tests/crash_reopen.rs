//! The write order, tested by performing its halves apart (spec §5, §14).
//!
//! A store is a SQLite index plus a blob tree, and the two commit
//! separately, so a process dying between them leaves one ahead of the
//! other. §14 fixes which one: the body lands before the row that
//! references it, so the worst a crash leaves is an orphan blob, which
//! the collector reclaims, and never a committed row pointing at bytes
//! that never arrived, which no operation can repair.
//!
//! Interruption is simulated by doing the halves separately rather than
//! by killing a process: the blob write is a public operation
//! (`PimdirBlobs::writer`), so the state a crash leaves is a state a test
//! can construct exactly, and reopening the store from disk is what turns
//! it back into an observation.
//!
//! Which side of the transaction the body is written on is deliberately
//! not asserted here. A blob write is not transactional, so staging ahead
//! of `BEGIN` and staging inside it leave the same files behind; what
//! separates them is how long the writer lock is held, which is not a
//! state a store can be read for.

use std::{io::Write, path::Path};

use io_pimdir::{
    change::PimdirWriteOp,
    collection::PimdirCollectionId,
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirSortKey, PimdirStatus,
    },
};
use io_pimdir::{
    client::producer::PimdirProducer,
    client::{PimdirError, PimdirSourceStore, PimdirStore},
    codec::PimdirAction,
};

const INBOX: &str = "INBOX";

fn store(dir: &Path) -> PimdirSourceStore {
    let store = PimdirStore::open(dir).unwrap().for_source("remote");
    store.ensure_collection(INBOX, "message/rfc822").unwrap();
    store
}

fn blob_of(dir: &Path, hash: &PimdirHash) -> std::path::PathBuf {
    dir.join("objects")
        .join(&hash.0[0..2])
        .join(&hash.0[2..4])
        .join(&hash.0)
}

/// Writes one body to the blob tree and stops there: the first half of
/// §14 step 1, and exactly what a process that dies before its
/// transaction leaves behind.
fn stage_body_only(store: &PimdirSourceStore, bytes: &[u8]) -> PimdirHash {
    let hash = store.hash(bytes);
    let blobs = store.blobs();
    let mut writer = blobs.writer().unwrap();
    writer.write_all(bytes).unwrap();
    writer.commit(&hash).unwrap();
    hash
}

fn placement(
    store: &PimdirSourceStore,
    handle: &str,
    link: &str,
    object: &[u8],
) -> PimdirPlacement {
    PimdirPlacement {
        collection: PimdirCollectionId(INBOX.into()),
        handle: PimdirHandle(handle.into()),
        link_id: Some(PimdirLinkId(link.into())),
        object: Some(store.hash(object)),
        level: PimdirLevel::Full,
        summary: None,
        sort_key: PimdirSortKey("k".into()),
        flags: PimdirFlags::default(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: None,
            object: Some(store.hash(object)),
        }),
        origin: None,
    }
}

/// Every hash the index holds that has no body on disk: the asymmetry the
/// write order exists to prevent, so this is expected to stay empty
/// whatever happened.
fn rows_without_bodies(store: &PimdirSourceStore, dir: &Path) -> Vec<String> {
    store
        .indexed_hashes()
        .unwrap()
        .into_iter()
        .filter(|hash| !blob_of(dir, &PimdirHash(hash.clone())).is_file())
        .collect()
}

/// A body on disk with no row is not a deleted one (spec §5): it survives
/// the crash, the reopen and every later write, until a collector is
/// asked to run.
#[test]
fn a_body_written_before_its_row_survives_the_crash_and_the_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let before = store(dir.path());
    let hash = stage_body_only(&before, b"streamed");
    assert!(blob_of(dir.path(), &hash).is_file());
    // The process dies here: the bytes are durable, nothing references
    // them, and no transaction ever opened.
    drop(before);

    let mut after = store(dir.path());
    assert!(
        blob_of(dir.path(), &hash).is_file(),
        "reopening a store is not a reclamation"
    );

    // Unrelated writes must not touch it either: no write reclaims.
    after
        .write(vec![
            PimdirWriteOp::StoreObject {
                object: PimdirObject {
                    hash: after.hash(b"other"),
                    size: 5,
                },
                body: Some(b"other".to_vec()),
            },
            PimdirWriteOp::UpsertPlacement(placement(&after, "1", "mid:a", b"other")),
        ])
        .unwrap();
    assert!(
        blob_of(dir.path(), &hash).is_file(),
        "a write reclaims nothing (spec §5)"
    );

    // The pattern the order exists for: the later batch indexes the body
    // it streamed earlier, carrying no bytes, and attaches it.
    after
        .write(vec![
            PimdirWriteOp::StoreObject {
                object: PimdirObject {
                    hash: hash.clone(),
                    size: b"streamed".len(),
                },
                body: None,
            },
            PimdirWriteOp::UpsertPlacement(placement(&after, "2", "mid:b", b"streamed")),
        ])
        .unwrap();

    assert_eq!(
        after.blobs().get(&hash).unwrap().as_deref(),
        Some(&b"streamed"[..]),
        "the body the first half wrote is the one the second half attached"
    );
    assert!(rows_without_bodies(&after, dir.path()).is_empty());
    assert!(after.refcount_drift().unwrap().is_empty());
}

/// A batch that fails inside its transaction leaves the bodies it staged
/// and no rows: an orphan blob, never a row without its body.
///
/// The refused rebind (spec §10) is the reachable failure that happens
/// after the bodies have landed, so it stands in for the crash between
/// §14 step 1 and the commit.
#[test]
fn a_failed_batch_leaves_an_orphan_and_never_a_bodiless_row() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = store(dir.path());

    store
        .write(vec![
            PimdirWriteOp::StoreObject {
                object: PimdirObject {
                    hash: store.hash(b"first"),
                    size: 5,
                },
                body: Some(b"first".to_vec()),
            },
            PimdirWriteOp::UpsertPlacement(placement(&store, "1", "mid:a", b"first")),
        ])
        .unwrap();

    // The same identity under a second handle: refused whole, after the
    // batch's body has already reached the blob tree.
    let refused = store.write(vec![
        PimdirWriteOp::StoreObject {
            object: PimdirObject {
                hash: store.hash(b"second"),
                size: 6,
            },
            body: Some(b"second".to_vec()),
        },
        PimdirWriteOp::UpsertPlacement(placement(&store, "9", "mid:a", b"second")),
    ]);
    assert!(matches!(refused, Err(PimdirError::Rebind { .. })));

    let orphan = store.hash(b"second");
    assert!(
        blob_of(dir.path(), &orphan).is_file(),
        "the staged body is on disk"
    );
    assert!(
        !store.indexed_hashes().unwrap().contains(&orphan.0),
        "and the transaction that would have indexed it rolled back"
    );
    assert!(rows_without_bodies(&store, dir.path()).is_empty());
    assert!(store.refcount_drift().unwrap().is_empty());

    // Reopening changes nothing, and the collector is what reclaims it.
    drop(store);
    let mut reopened = self::store(dir.path());
    assert!(blob_of(dir.path(), &orphan).is_file());
    let collected = reopened.collect_garbage().unwrap();
    assert_eq!((collected.objects, collected.blobs), (0, 1));
    assert!(!blob_of(dir.path(), &orphan).is_file());
    assert!(
        blob_of(dir.path(), &reopened.hash(b"first")).is_file(),
        "the committed batch's body is untouched"
    );
}

/// The producer's window, in both of the states a crash can leave it in
/// (spec §8, §15.1): the body written and the row not yet, and the body
/// written with the row committed.
#[test]
fn a_producer_body_survives_with_and_without_the_row_that_pins_it() {
    let dir = tempfile::tempdir().unwrap();
    let owner = store(dir.path());

    // Half one, then the crash: a body in the tree that nothing names.
    let unpinned = stage_body_only(&owner, b"unpinned");

    // Half two, complete: another body, and the queue row that pins it.
    let pinned = stage_body_only(&owner, b"pinned");
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    producer
        .enqueue(
            INBOX,
            &PimdirAction::Add {
                link_id: Some(PimdirLinkId("mid:queued".into())),
                flags: PimdirFlags::default(),
                object: Some(pinned.clone()),
                handle: None,
            },
            Some(b"pinned".len() as u64),
        )
        .unwrap();
    drop(producer);

    drop(owner);
    let mut reopened = store(dir.path());
    assert!(reopened.refcount_drift().unwrap().is_empty());
    assert_eq!(reopened.pending_actions(INBOX).unwrap().len(), 1);

    // The pin holds across the reopen and the collection; the body that
    // never reached a row is the orphan the collector exists for.
    let collected = reopened.collect_garbage().unwrap();
    assert_eq!(
        (collected.objects, collected.blobs),
        (0, 1),
        "one orphan file, and no row taken"
    );
    assert!(!blob_of(dir.path(), &unpinned).is_file());
    assert_eq!(
        reopened.blobs().get(&pinned).unwrap().as_deref(),
        Some(&b"pinned"[..]),
        "a queued body is pinned between enqueue and apply (spec §15)"
    );
    assert!(rows_without_bodies(&reopened, dir.path()).is_empty());
}
