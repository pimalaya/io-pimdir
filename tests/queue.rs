//! The action queue and collection generations: a producer's enqueue against
//! a real temp store, the owner's drain per action kind, parking, refcount
//! pinning of queued bodies, generation bumps, and the refusal of a store
//! stamped with a schema version this crate does not service.

use std::{io::Write, path::Path};

use io_pimdir::{
    PimdirBlobs, PimdirError, PimdirProducer, PimdirStore, codec::PimdirAction,
    hash::PimdirHashAlgo, sql,
};
use io_replica::{
    change::ReplicaWriteOp,
    client::ReplicaStorage,
    collection::ReplicaCollectionId,
    object::{ReplicaHash, ReplicaObject},
    placement::{
        ReplicaBase, ReplicaFlags, ReplicaHandle, ReplicaLevel, ReplicaLinkId, ReplicaMeta,
        ReplicaPlacement, ReplicaStatus,
    },
};

const NOW: &str = "2026-08-07T00:00:00Z";

fn inbox() -> ReplicaCollectionId {
    ReplicaCollectionId("INBOX".into())
}

/// A hydrated, linked placement with a matching base (so it projects clean).
fn placement(handle: &str, link: &str, hash: &str, flags: &[&str]) -> ReplicaPlacement {
    let flags = ReplicaFlags::from_iter(flags.iter().copied());
    ReplicaPlacement {
        sort_key: Default::default(),
        collection: inbox(),
        handle: ReplicaHandle(handle.into()),
        link_id: Some(ReplicaLinkId(link.into())),
        object: Some(ReplicaHash(hash.into())),
        level: ReplicaLevel::Full,
        meta: None,
        flags: flags.clone(),
        status: ReplicaStatus::Clean,
        conflict_revision: None,
        base: Some(ReplicaBase {
            flags,
            revision: None,
            object: Some(ReplicaHash(hash.into())),
        }),
        origin: None,
    }
}

fn store_object(hash: &str, body: &[u8]) -> ReplicaWriteOp {
    ReplicaWriteOp::StoreObject {
        object: ReplicaObject {
            hash: ReplicaHash(hash.into()),
            size: body.len(),
        },
        body: Some(body.to_vec()),
    }
}

fn blob_exists(dir: &Path, hash: &str) -> bool {
    dir.join("objects")
        .join(&hash[0..2])
        .join(&hash[2..4])
        .join(hash)
        .exists()
}

/// Seeds a one-item INBOX and returns `(store, seq)` for the item.
fn seeded(dir: &Path) -> (PimdirStore, i64) {
    let mut store = PimdirStore::open(dir, "local").unwrap();
    store
        .write(vec![
            store_object("cafebabe", b"abc"),
            ReplicaWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe", &["\\Seen"])),
        ])
        .unwrap();
    let seq = store.list_items("INBOX", None, 10).unwrap()[0].seq;
    (store, seq)
}

/// Streams `body` into the blob store under `hash` (the producer's blob-first
/// step) and returns its size.
fn write_blob(dir: &Path, hash: &str, body: &[u8]) -> u64 {
    let blobs = PimdirBlobs::open(dir, PimdirHashAlgo::default());
    let mut writer = blobs.writer().unwrap();
    writer.write_all(body).unwrap();
    writer.commit(&ReplicaHash(hash.into())).unwrap()
}

#[test]
fn a_queued_add_round_trips_into_a_staged_item() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, _) = seeded(dir.path());

    // The producer writes the blob durably first, then enqueues in one
    // transaction; the pending action is visible to its own reads (overlay).
    let size = write_blob(dir.path(), "beef0000", b"new body");
    let mut producer = PimdirProducer::open(dir.path(), "smtp").unwrap();
    let add = PimdirAction::Add {
        link_id: Some(ReplicaLinkId("mid:new".into())),
        flags: ReplicaFlags::from_iter(["\\Draft"]),
        object: Some(ReplicaHash("beef0000".into())),
        meta: Some(ReplicaMeta("{\"subject\":\"hi\",\"v\":1}".into())),
        handle: Some(ReplicaHandle("draft-1".into())),
    };
    producer.enqueue("INBOX", &add, Some(size), NOW).unwrap();

    let pending = producer.pending_actions("INBOX").unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].action, add);
    assert_eq!(pending[0].producer, "smtp");
    assert_eq!(store.queued_collections().unwrap(), ["INBOX"]);

    // The owner drains: the item lands, staged as a pending push.
    let report = store.drain_collection("INBOX").unwrap();
    assert_eq!((report.applied, report.parked), (1, 0));
    assert!(store.pending_actions("INBOX").unwrap().is_empty());
    assert!(store.queued_collections().unwrap().is_empty());

    let items = store.list_items("INBOX", None, 10).unwrap();
    let item = items.iter().find(|i| i.link_id.0 == "mid:new").unwrap();
    assert_eq!(item.object, Some(ReplicaHash("beef0000".into())));
    assert_eq!(item.level, ReplicaLevel::Full);
    assert!(item.flags.contains("\\Draft"));
    assert_eq!(
        item.meta.as_ref().unwrap().0,
        "{\"subject\":\"hi\",\"v\":1}"
    );

    // Projected as a pending, base-less push for the sync layer to derive
    // (the same shape a staged in-process Add projects).
    let projected = store.load(&inbox()).unwrap().placements;
    let created = projected
        .iter()
        .find(|p| p.link_id.as_ref().is_some_and(|l| l.0 == "mid:new"))
        .unwrap();
    assert_ne!(created.status, ReplicaStatus::Clean, "a pending push");
    assert!(created.base.is_none(), "no prior sync: an append push");
}

#[test]
fn a_duplicate_add_parks_instead_of_clobbering() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, _) = seeded(dir.path());

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let add = PimdirAction::Add {
        link_id: Some(ReplicaLinkId("mid:a".into())),
        flags: ReplicaFlags::default(),
        object: None,
        meta: None,
        handle: None,
    };
    producer.enqueue("INBOX", &add, None, NOW).unwrap();

    let report = store.drain_collection("INBOX").unwrap();
    assert_eq!((report.applied, report.parked), (0, 1));
    let parked = store.parked_actions().unwrap();
    assert_eq!(parked.len(), 1);
    assert!(parked[0].error.contains("already present"), "{parked:?}");
    assert_eq!(parked[0].attempts, 1);
    // The live item is untouched.
    assert!(
        store.list_items("INBOX", None, 10).unwrap()[0]
            .flags
            .contains("\\Seen")
    );
}

#[test]
fn set_flags_is_absolute_and_reapplies_idempotently() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq) = seeded(dir.path());
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let set = PimdirAction::SetFlags {
        seq,
        flags: ReplicaFlags::from_iter(["\\Flagged"]),
    };

    producer.enqueue("INBOX", &set, None, NOW).unwrap();
    let report = store.drain_collection("INBOX").unwrap();
    assert_eq!((report.applied, report.parked), (1, 0));

    // Absolute replacement: the old flag is gone, not merged.
    let item = store.get_item("INBOX", seq).unwrap().unwrap();
    assert_eq!(item.flags, ReplicaFlags::from_iter(["\\Flagged"]));

    // Reapplying the same absolute set is a no-op, never an error.
    producer.enqueue("INBOX", &set, None, NOW).unwrap();
    let report = store.drain_collection("INBOX").unwrap();
    assert_eq!((report.applied, report.parked), (1, 0));
    let item = store.get_item("INBOX", seq).unwrap().unwrap();
    assert_eq!(item.flags, ReplicaFlags::from_iter(["\\Flagged"]));
}

#[test]
fn remove_hides_the_item_and_an_absent_remove_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq) = seeded(dir.path());
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();

    producer
        .enqueue("INBOX", &PimdirAction::Remove { seq }, None, NOW)
        .unwrap();
    let report = store.drain_collection("INBOX").unwrap();
    assert_eq!((report.applied, report.parked), (1, 0));

    // Hidden from reads, kept as a tombstone for the sync to push.
    assert!(store.get_item("INBOX", seq).unwrap().is_none());
    let projected = store.load(&inbox()).unwrap().placements;
    assert_eq!(projected[0].status, ReplicaStatus::Tombstone);

    // Removing it again is success (idempotent), not a park.
    producer
        .enqueue("INBOX", &PimdirAction::Remove { seq }, None, NOW)
        .unwrap();
    let report = store.drain_collection("INBOX").unwrap();
    assert_eq!((report.applied, report.parked), (1, 0));
    assert!(store.parked_actions().unwrap().is_empty());
}

#[test]
fn copy_fills_the_target_and_move_also_empties_the_source() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path(), "local").unwrap();
    store
        .write(vec![
            store_object("cafebabe", b"a"),
            store_object("cafebabf", b"b"),
            ReplicaWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe", &[])),
            ReplicaWriteOp::UpsertPlacement(placement("2", "mid:b", "cafebabf", &[])),
        ])
        .unwrap();
    let seq_of = |store: &PimdirStore, link: &str| {
        store
            .list_items("INBOX", None, 10)
            .unwrap()
            .into_iter()
            .find(|i| i.link_id.0 == link)
            .unwrap()
            .seq
    };
    let seq_a = seq_of(&store, "mid:a");
    let seq_b = seq_of(&store, "mid:b");

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::Copy {
                seq: seq_a,
                to: ReplicaCollectionId("Backup".into()),
            },
            None,
            NOW,
        )
        .unwrap();
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::Move {
                seq: seq_b,
                to: ReplicaCollectionId("Archive".into()),
            },
            None,
            NOW,
        )
        .unwrap();

    let report = store.drain_collection("INBOX").unwrap();
    assert_eq!((report.applied, report.parked), (2, 0));

    // Copy: source keeps the item, the target gains it under the same seq.
    assert_eq!(store.count_items("INBOX").unwrap(), 1, "mid:b moved away");
    let backup = store.list_items("Backup", None, 10).unwrap();
    assert_eq!(backup.len(), 1);
    assert_eq!(backup[0].link_id.0, "mid:a");
    assert_eq!(backup[0].seq, seq_a, "a message keeps one public id");

    // Move: the source empties, the target fills.
    let archive = store.list_items("Archive", None, 10).unwrap();
    assert_eq!(archive.len(), 1);
    assert_eq!(archive[0].link_id.0, "mid:b");
    assert_eq!(archive[0].seq, seq_b);
    assert!(store.get_item("INBOX", seq_b).unwrap().is_none());
}

#[test]
fn update_repoints_the_body() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq) = seeded(dir.path());

    let size = write_blob(dir.path(), "beef0000", b"edited body");
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::Update {
                seq,
                object: ReplicaHash("beef0000".into()),
                meta: None,
            },
            Some(size),
            NOW,
        )
        .unwrap();

    let report = store.drain_collection("INBOX").unwrap();
    assert_eq!((report.applied, report.parked), (1, 0));

    let item = store.get_item("INBOX", seq).unwrap().unwrap();
    assert_eq!(item.object, Some(ReplicaHash("beef0000".into())));
    // The old body survives: the sync base still references it.
    assert!(blob_exists(dir.path(), "cafebabe"));
    assert!(blob_exists(dir.path(), "beef0000"));
    // Projected dirty, so the next sync pushes the edit.
    let projected = store.load(&inbox()).unwrap().placements;
    assert_eq!(projected[0].status, ReplicaStatus::Dirty);
}

#[test]
fn a_parked_action_does_not_block_later_actions() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq) = seeded(dir.path());
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();

    // First an unappliable action (unknown seq), then a valid one behind it.
    producer
        .enqueue("INBOX", &PimdirAction::Remove { seq: 9999 }, None, NOW)
        .unwrap();
    // NOTE: remove-of-absent succeeds, so use set-flags for the parking case.
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::SetFlags {
                seq: 9999,
                flags: ReplicaFlags::default(),
            },
            None,
            NOW,
        )
        .unwrap();
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::SetFlags {
                seq,
                flags: ReplicaFlags::from_iter(["\\Answered"]),
            },
            None,
            NOW,
        )
        .unwrap();

    let report = store.drain_collection("INBOX").unwrap();
    assert_eq!((report.applied, report.parked), (2, 1));

    // The later action applied despite the parked one before it.
    let item = store.get_item("INBOX", seq).unwrap().unwrap();
    assert!(item.flags.contains("\\Answered"));

    // The parked row is recorded, queryable, and skipped by the next pass.
    let parked = store.parked_actions().unwrap();
    assert_eq!(parked.len(), 1);
    assert_eq!(parked[0].collection, "INBOX");
    assert_eq!(parked[0].action, "set-flags");
    assert!(parked[0].error.contains("unknown seq"), "{parked:?}");
    let report = store.drain_collection("INBOX").unwrap();
    assert_eq!((report.applied, report.parked), (0, 0));
    assert_eq!(store.parked_actions().unwrap().len(), 1, "never deleted");
}

#[test]
fn gc_never_sweeps_a_queued_body() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seeded_seq) = seeded(dir.path());

    // A producer stages a body and enqueues the add that references it.
    let size = write_blob(dir.path(), "beef0000", b"queued body");
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::Add {
                link_id: Some(ReplicaLinkId("mid:new".into())),
                flags: ReplicaFlags::default(),
                object: Some(ReplicaHash("beef0000".into())),
                meta: None,
                handle: Some(ReplicaHandle("draft-1".into())),
            },
            Some(size),
            NOW,
        )
        .unwrap();

    // The owner runs a write batch whose GC sweeps a genuinely orphaned body
    // (the seeded item's, retired then purged out of retention): the queued
    // body must survive it, pinned by its row.
    store
        .write(vec![ReplicaWriteOp::DropPlacement {
            collection: inbox(),
            handle: ReplicaHandle("1".into()),
        }])
        .unwrap();
    assert!(store.purge(&inbox(), seeded_seq).unwrap());
    assert!(!blob_exists(dir.path(), "cafebabe"), "the orphan is GC'd");
    assert!(
        blob_exists(dir.path(), "beef0000"),
        "the queued body is not"
    );

    // Draining hands the pin over to the applied item.
    let report = store.drain_collection("INBOX").unwrap();
    assert_eq!((report.applied, report.parked), (1, 0));
    assert!(blob_exists(dir.path(), "beef0000"), "now the item pins it");

    // The hand-over was exact (+1 item, -1 queue): retiring and purging the
    // item's only placement orphans the body, so it is swept, with no
    // leaked queue pin.
    let seq = store.seq_for_link("INBOX", "mid:new").unwrap().unwrap();
    store
        .write(vec![ReplicaWriteOp::DropPlacement {
            collection: inbox(),
            handle: ReplicaHandle("draft-1".into()),
        }])
        .unwrap();
    assert!(store.purge(&inbox(), seq).unwrap());
    assert!(!blob_exists(dir.path(), "beef0000"), "no refcount leak");
}

#[test]
fn an_unknown_kind_is_skipped_and_never_blocks_the_queue() {
    // The store defines no semantics for a `submit` intent, but another owner
    // does: skipping leaves it pending for that owner, where parking would
    // claim it can never be applied by anyone.
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq) = seeded(dir.path());
    let mut producer = PimdirProducer::open(dir.path(), "himalaya").unwrap();

    let size = write_blob(dir.path(), "beef0000", b"a message to send");
    let submit = PimdirAction::Unknown {
        kind: "submit".into(),
        payload: "{\"v\":1,\"object\":\"beef0000\",\"to\":[\"a@b.c\"]}".into(),
        object_hash: Some(ReplicaHash("beef0000".into())),
    };
    let id = producer.enqueue("INBOX", &submit, Some(size), NOW).unwrap();
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::SetFlags {
                seq,
                flags: ReplicaFlags::from_iter(["\\Answered"]),
            },
            None,
            NOW,
        )
        .unwrap();

    let report = store.drain_collection("INBOX").unwrap();
    assert_eq!((report.applied, report.parked, report.skipped), (1, 0, 1));

    // The action behind it applied all the same.
    assert!(
        store
            .get_item("INBOX", seq)
            .unwrap()
            .unwrap()
            .flags
            .contains("\\Answered")
    );

    // The intent is still pending, never parked, and still readable whole by
    // the owner that can perform it; its body is still pinned.
    assert!(store.parked_actions().unwrap().is_empty());
    let pending = store.pending_actions("INBOX").unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, id);
    assert_eq!(pending[0].action, submit);
    assert!(blob_exists(dir.path(), "beef0000"));

    // A second pass skips it again rather than accumulating attempts.
    let report = store.drain_collection("INBOX").unwrap();
    assert_eq!((report.applied, report.parked, report.skipped), (0, 0, 1));
    assert_eq!(store.pending_actions("INBOX").unwrap()[0].attempts, 0);
}

#[test]
fn an_acknowledged_action_releases_its_queued_body() {
    // The other half of skip-not-park: the owner that performed the intent out
    // of band takes the row away, and with it the pin on the body it carried.
    let dir = tempfile::tempdir().unwrap();
    let (mut store, _) = seeded(dir.path());
    let mut producer = PimdirProducer::open(dir.path(), "himalaya").unwrap();

    let size = write_blob(dir.path(), "beef0000", b"sent already");
    let id = producer
        .enqueue(
            "INBOX",
            &PimdirAction::Unknown {
                kind: "submit".into(),
                payload: "{\"v\":1,\"object\":\"beef0000\"}".into(),
                object_hash: Some(ReplicaHash("beef0000".into())),
            },
            Some(size),
            NOW,
        )
        .unwrap();
    assert!(blob_exists(dir.path(), "beef0000"));

    assert!(store.drop_action(id).unwrap());
    assert!(store.pending_actions("INBOX").unwrap().is_empty());
    assert!(
        !blob_exists(dir.path(), "beef0000"),
        "the pin went with the row"
    );

    // Acknowledging a row that is already gone reports nothing to drop.
    assert!(!store.drop_action(id).unwrap());
}

#[test]
fn a_failed_action_retries_until_it_is_parked() {
    // The two failure shapes an owner reports itself: transient (still
    // pending, attempts advancing) and permanent (parked with the reason).
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq) = seeded(dir.path());
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let id = producer
        .enqueue("INBOX", &PimdirAction::Remove { seq }, None, NOW)
        .unwrap();

    store.fail_action(id, None).unwrap();
    store.fail_action(id, None).unwrap();
    let pending = store.pending_actions("INBOX").unwrap();
    assert_eq!(pending.len(), 1, "a transient failure stays pending");
    assert_eq!(pending[0].attempts, 2);

    store.fail_action(id, Some("the channel is gone")).unwrap();
    assert!(store.pending_actions("INBOX").unwrap().is_empty());
    let parked = store.parked_actions().unwrap();
    assert_eq!(parked.len(), 1);
    assert_eq!(parked[0].id, id);
    assert_eq!(parked[0].attempts, 3, "the parking attempt counts too");
    assert_eq!(parked[0].error, "the channel is gone");

    // A parked row is still cancellable, and an unknown id is a no-op.
    assert!(store.drop_action(id).unwrap());
    assert!(store.parked_actions().unwrap().is_empty());
    store.fail_action(id, Some("gone")).unwrap();
}

#[test]
fn a_generation_bump_is_visible_to_a_reader() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path(), "local").unwrap();
    store.ensure_collection("INBOX", "message/rfc822").unwrap();

    // The epoch starts at 1 and ordinary writes never bump it.
    assert_eq!(store.generation("INBOX").unwrap(), Some(1));
    store
        .write(vec![
            store_object("cafebabe", b"abc"),
            ReplicaWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe", &[])),
        ])
        .unwrap();
    assert_eq!(store.generation("INBOX").unwrap(), Some(1));

    // A handle-space rebuild (rekey) bumps it in the same transaction: the
    // item is re-bound under its new handle and the epoch advances together.
    let mut rekeyed = placement("101", "mid:a", "cafebabe", &[]);
    rekeyed.status = ReplicaStatus::Clean;
    let generation = store
        .write_rekeyed(
            "INBOX",
            vec![
                ReplicaWriteOp::DropPlacement {
                    collection: inbox(),
                    handle: ReplicaHandle("1".into()),
                },
                ReplicaWriteOp::UpsertPlacement(rekeyed),
            ],
        )
        .unwrap();
    assert_eq!(generation, 2);

    // A second reader handle over the same files observes the new epoch.
    let reader = PimdirStore::open(dir.path(), "local").unwrap();
    assert_eq!(reader.generation("INBOX").unwrap(), Some(2));
    let collections = reader.list_collections().unwrap();
    assert_eq!(collections[0].generation, 2);
    // An unknown collection has no epoch.
    assert_eq!(reader.generation("nope").unwrap(), None);
}

#[test]
fn a_store_stamped_with_a_higher_version_is_refused() {
    let dir = tempfile::tempdir().unwrap();

    // A fresh store lands at the current draft schema version, with
    // `user_version` and `store_meta.version` in agreement.
    {
        let _store = PimdirStore::open(dir.path(), "local").unwrap();
        let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
        let user_version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        let meta_version: i64 = conn
            .query_row("SELECT version FROM store_meta WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!((user_version, meta_version), (sql::VERSION, sql::VERSION));
    }

    // Stamp the store with a higher schema version, as a newer draft would.
    {
        let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
        conn.pragma_update(None, "user_version", sql::VERSION + 1)
            .unwrap();
    }

    // Every opener refuses it rather than half-reading: the owner (which
    // never migrates; a draft store is recreated), the producer and the
    // read-only reader (which additionally refuse any version but the
    // current one).
    for result in [
        PimdirStore::open(dir.path(), "local").map(drop),
        PimdirProducer::open(dir.path(), "test").map(drop),
        PimdirStore::open_read_only(dir.path(), "local").map(drop),
    ] {
        match result {
            Err(PimdirError::Version { found }) => assert_eq!(found, sql::VERSION + 1),
            Err(other) => panic!("expected a version refusal, got {other:?}"),
            Ok(()) => panic!("expected a version refusal, got an open store"),
        }
    }

    // A producer also refuses a store the owner has not created yet
    // (`user_version` 0): it never creates the schema.
    let fresh = tempfile::tempdir().unwrap();
    rusqlite::Connection::open(fresh.path().join("pimdir.db")).unwrap();
    match PimdirProducer::open(fresh.path(), "test") {
        Err(PimdirError::Version { found: 0 }) => {}
        Err(other) => panic!("expected a version refusal, got {other:?}"),
        Ok(_) => panic!("expected a version refusal, got a producer"),
    }
}
