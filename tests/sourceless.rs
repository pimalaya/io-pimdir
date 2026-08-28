//! A store handle names a source only where an operation acts as one.
//!
//! The operator surface, the client reads, retention and its purge and a
//! queued action cancelled rather than applied, reads no source, so it
//! opens a store that carries none. What the source-less handle protects
//! is the store: a name invented to satisfy a constructor is a side that
//! never synced anything, one write away from being recorded as one.

use io_pimdir::{PimdirProducer, PimdirStore, codec::PimdirAction};
use io_replica::{
    change::{ReplicaDropReason, ReplicaWriteOp},
    client::ReplicaStorage,
    collection::ReplicaCollectionId,
    object::{ReplicaHash, ReplicaObject},
    placement::{
        ReplicaBase, ReplicaFlags, ReplicaHandle, ReplicaLevel, ReplicaLinkId, ReplicaMeta,
        ReplicaPlacement, ReplicaStatus,
    },
};

fn inbox() -> ReplicaCollectionId {
    ReplicaCollectionId("INBOX".into())
}

fn placement(handle: &str, link: &str, hash: &str) -> ReplicaPlacement {
    ReplicaPlacement {
        sort_key: Default::default(),
        collection: inbox(),
        handle: ReplicaHandle(handle.into()),
        link_id: Some(ReplicaLinkId(link.into())),
        object: Some(ReplicaHash(hash.into())),
        level: ReplicaLevel::Full,
        meta: Some(ReplicaMeta("{\"v\":1}".into())),
        flags: ReplicaFlags::default(),
        status: ReplicaStatus::Clean,
        conflict_revision: None,
        base: Some(ReplicaBase {
            flags: ReplicaFlags::default(),
            revision: None,
            object: Some(ReplicaHash(hash.into())),
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
        ReplicaWriteOp::StoreObject {
            object: ReplicaObject {
                hash: ReplicaHash("cafebabe".into()),
                size: 4,
            },
            body: Some(b"body".to_vec()),
        },
        ReplicaWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe")),
    ])
    .unwrap();
    let seq = sync.list_items("INBOX", None, 10).unwrap()[0].seq;
    sync.write(vec![ReplicaWriteOp::DropPlacement {
        collection: inbox(),
        handle: ReplicaHandle("1".into()),
        reason: ReplicaDropReason::Deleted,
    }])
    .unwrap();
    drop(sync);

    let id = PimdirProducer::open(dir.path(), "test")
        .unwrap()
        .enqueue(
            "INBOX",
            &PimdirAction::Remove { seq },
            None,
            "2026-01-01T00:00:00.000Z",
        )
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
