//! The read-only open: reads an owner-created store, refuses a missing or
//! stale-version one, and its write path fails at the SQLite layer.

use io_pimdir::{PimdirError, PimdirStore};
use io_replica::{
    change::{ReplicaDropReason, ReplicaWriteOp},
    client::ReplicaStorage,
    collection::ReplicaCollectionId,
    object::{ReplicaHash, ReplicaObject},
    placement::{
        ReplicaBase, ReplicaFlags, ReplicaHandle, ReplicaLevel, ReplicaLinkId, ReplicaPlacement,
        ReplicaStatus,
    },
};

#[test]
fn read_only_open_reads_but_never_writes_or_creates() {
    let dir = tempfile::tempdir().unwrap();

    // NOTE: no database yet, and a reader never creates one.
    assert!(matches!(
        PimdirStore::open_read_only(dir.path()),
        Err(PimdirError::Sql(_))
    ));

    // the owner creates and fills the store
    let mut owner = PimdirStore::open(dir.path()).unwrap().for_source("left");
    owner.ensure_collection("INBOX", "message/rfc822").unwrap();
    let flags = ReplicaFlags::from_iter(["\\Seen"]);
    owner
        .write(vec![
            ReplicaWriteOp::StoreObject {
                object: ReplicaObject {
                    hash: ReplicaHash("cafe".into()),
                    size: 4,
                },
                body: Some(b"body".to_vec()),
            },
            ReplicaWriteOp::UpsertPlacement(ReplicaPlacement {
                sort_key: Default::default(),
                collection: ReplicaCollectionId("INBOX".into()),
                handle: ReplicaHandle("1".into()),
                link_id: Some(ReplicaLinkId("mid:a".into())),
                object: Some(ReplicaHash("cafe".into())),
                level: ReplicaLevel::Full,
                meta: None,
                flags: flags.clone(),
                status: ReplicaStatus::Clean,
                conflict_revision: None,
                base: Some(ReplicaBase {
                    flags,
                    revision: None,
                    object: Some(ReplicaHash("cafe".into())),
                }),
                origin: None,
                ambiguous_handles: Vec::new(),
            }),
        ])
        .unwrap();

    // the reader sees the shared truth through the same read surface
    let reader = PimdirStore::open_read_only(dir.path()).unwrap();
    let collections = reader.list_collections().unwrap();
    assert_eq!(collections.len(), 1);
    assert_eq!(collections[0].id, "INBOX");
    assert_eq!(collections[0].generation, 1);
    let items = reader.list_items("INBOX", None, 10).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].link_id, ReplicaLinkId("mid:a".into()));

    // NOTE: a write through the read-only handle fails at the SQLite
    // layer rather than mutating the owner's store.
    assert!(reader.ensure_collection("Other", "message/rfc822").is_err());
    let mut reader = reader.for_source("left");
    assert!(
        reader
            .write(vec![ReplicaWriteOp::DropPlacement {
                collection: ReplicaCollectionId("INBOX".into()),
                handle: ReplicaHandle("1".into()),
                reason: ReplicaDropReason::Deleted,
            }])
            .is_err()
    );
}
