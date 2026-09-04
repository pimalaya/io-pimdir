//! A store handle names a source only where an operation acts as one.
//!
//! The operator surface, the client reads, retention and its purge and a
//! queued action cancelled rather than applied, reads no source, so it
//! opens a store that carries none. What the source-less handle protects
//! is the store: a name invented to satisfy a constructor is a side that
//! never synced anything, one write away from being recorded as one.

use io_pimdir::{
    change::{PimdirDropReason, PimdirWriteOp},
    collection::PimdirCollectionId,
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
};
use io_pimdir::{client::PimdirStore, client::producer::PimdirProducer, codec::PimdirAction};

fn inbox() -> PimdirCollectionId {
    PimdirCollectionId("INBOX".into())
}

fn placement(handle: &str, link: &str, hash: &str) -> PimdirPlacement {
    PimdirPlacement {
        sort_key: Default::default(),
        collection: inbox(),
        handle: PimdirHandle(handle.into()),
        link_id: Some(PimdirLinkId(link.into())),
        object: Some(PimdirHash(hash.into())),
        level: PimdirLevel::Full,
        summary: None,
        flags: PimdirFlags::default(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: None,
            object: Some(PimdirHash(hash.into())),
        }),
        origin: None,
    }
}

/// Everything an operator does to a store without naming a side: read the
/// collection, cancel a queued action, purge what retention holds.
#[test]
fn a_source_less_handle_reads_purges_and_cancels() {
    let dir = tempfile::tempdir().unwrap();

    // the one role that does name a side seeds the item, then retires it
    let mut sync = PimdirStore::open(dir.path()).unwrap().for_source("remote");
    sync.write(vec![
        PimdirWriteOp::StoreObject {
            object: PimdirObject {
                hash: PimdirHash("cafebabe".into()),
                size: 4,
            },
            body: Some(b"body".to_vec()),
        },
        PimdirWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe")),
    ])
    .unwrap();
    let seq = sync.list_items("INBOX", None, 10).unwrap()[0].seq;
    sync.write(vec![PimdirWriteOp::DropPlacement {
        collection: inbox(),
        handle: PimdirHandle("1".into()),
        reason: PimdirDropReason::Deleted,
    }])
    .unwrap();
    drop(sync);

    let id = PimdirProducer::open(dir.path(), "test")
        .unwrap()
        .enqueue("INBOX", &PimdirAction::Remove { seq }, None)
        .unwrap();

    let mut store = PimdirStore::open(dir.path()).unwrap();
    assert_eq!(store.list_collections().unwrap().len(), 1);
    assert!(store.list_items("INBOX", None, 10).unwrap().is_empty());
    assert_eq!(store.list_retained(&inbox(), None, 10).unwrap().len(), 1);
    assert!(store.drop_action(id).unwrap());
    assert!(store.pending_actions("INBOX").unwrap().is_empty());
    assert!(store.purge(&inbox(), seq).unwrap());
    assert_eq!(store.count_retained(&inbox()).unwrap(), 0);
}

/// A store an operator opened, read and swept still syncs no source.
#[test]
fn an_operator_pass_records_no_source() {
    let dir = tempfile::tempdir().unwrap();

    let mut store = PimdirStore::open(dir.path()).unwrap();
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    assert!(store.list_items("INBOX", None, 10).unwrap().is_empty());
    assert!(!store.purge(&inbox(), 1).unwrap());
    assert!(!store.drop_action(1).unwrap());
    assert_eq!(
        store
            .purge_retained_before("2030-01-01T00:00:00.000Z")
            .unwrap()
            .items,
        0
    );

    assert!(store.distinct_sources().unwrap().is_empty());
}
