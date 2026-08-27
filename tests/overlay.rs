//! The reader role and its pending-action overlay (spec §8, §15.4): what
//! a frontend sees between staging a write and the owner applying it.

use std::fs::File;

use fs4::FileExt;
use io_pimdir::{PimdirError, PimdirReader, PimdirStore, codec::PimdirAction};
use io_replica::{
    change::ReplicaWriteOp,
    client::ReplicaStorage,
    collection::ReplicaCollectionId,
    object::{ReplicaHash, ReplicaObject},
    placement::{
        ReplicaBase, ReplicaFlags, ReplicaHandle, ReplicaLevel, ReplicaLinkId, ReplicaPlacement,
        ReplicaSortKey, ReplicaStatus,
    },
};

const NOW: &str = "2026-08-27T00:00:00Z";

/// A hydrated, linked placement with a matching base, so it projects clean.
fn placement(collection: &str, handle: &str, link: &str, key: &str) -> ReplicaPlacement {
    let flags = ReplicaFlags::from_iter(["\\Seen"]);
    ReplicaPlacement {
        sort_key: ReplicaSortKey(key.into()),
        collection: ReplicaCollectionId(collection.into()),
        handle: ReplicaHandle(handle.into()),
        link_id: Some(ReplicaLinkId(link.into())),
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
    }
}

/// An owner-created store holding `INBOX` (three items) and an empty
/// `Archive`, with the blob every item points at.
fn store(dir: &std::path::Path) -> Vec<i64> {
    let mut owner = PimdirStore::open(dir).unwrap().for_source("left");
    owner.ensure_collection("INBOX", "message/rfc822").unwrap();
    owner
        .ensure_collection("Archive", "message/rfc822")
        .unwrap();
    owner
        .write(vec![
            ReplicaWriteOp::StoreObject {
                object: ReplicaObject {
                    hash: ReplicaHash("cafe".into()),
                    size: 4,
                },
                body: Some(b"body".to_vec()),
            },
            ReplicaWriteOp::UpsertPlacement(placement("INBOX", "1", "mid:a", "2026-01-01")),
            ReplicaWriteOp::UpsertPlacement(placement("INBOX", "2", "mid:b", "2026-01-02")),
            ReplicaWriteOp::UpsertPlacement(placement("INBOX", "3", "mid:c", "2026-01-03")),
        ])
        .unwrap();

    let reader = PimdirReader::open(dir).unwrap();
    reader
        .list_items("INBOX", None, 10)
        .unwrap()
        .into_iter()
        .map(|item| item.seq)
        .collect()
}

/// Stages one action against a collection, as a producer would.
fn enqueue(dir: &std::path::Path, collection: &str, action: &PimdirAction) {
    let mut producer = io_pimdir::PimdirProducer::open(dir, "test").unwrap();
    producer.enqueue(collection, action, None, NOW).unwrap();
}

#[test]
fn a_staged_flag_shows_on_the_overlaid_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let seqs = store(dir.path());

    enqueue(
        dir.path(),
        "INBOX",
        &PimdirAction::SetFlags {
            seq: seqs[0],
            flags: ReplicaFlags::from_iter(["\\Flagged"]),
        },
    );

    let raw = PimdirReader::open(dir.path()).unwrap();
    assert!(!raw.overlays_pending());
    let item = raw.get_item("INBOX", seqs[0]).unwrap().unwrap();
    assert!(item.flags.contains("\\Seen"));
    assert!(!item.flags.contains("\\Flagged"));

    let overlaid = PimdirReader::open(dir.path()).unwrap().with_pending();
    let item = overlaid.get_item("INBOX", seqs[0]).unwrap().unwrap();
    assert!(item.flags.contains("\\Flagged"));
    // NOTE: `set-flags` is absolute rather than a delta (spec §15.3), so
    // the staged set replaces the stored one instead of adding to it.
    assert!(!item.flags.contains("\\Seen"));
}

#[test]
fn two_staged_actions_on_one_item_fold_in_append_order() {
    let dir = tempfile::tempdir().unwrap();
    let seqs = store(dir.path());

    enqueue(
        dir.path(),
        "INBOX",
        &PimdirAction::SetFlags {
            seq: seqs[0],
            flags: ReplicaFlags::from_iter(["\\Flagged"]),
        },
    );
    enqueue(
        dir.path(),
        "INBOX",
        &PimdirAction::SetFlags {
            seq: seqs[0],
            flags: ReplicaFlags::from_iter(["\\Answered"]),
        },
    );

    let overlaid = PimdirReader::open(dir.path()).unwrap().with_pending();
    let item = overlaid.get_item("INBOX", seqs[0]).unwrap().unwrap();
    assert!(item.flags.contains("\\Answered"));
    assert!(!item.flags.contains("\\Flagged"));
}

#[test]
fn a_staged_removal_leaves_the_listing_and_the_count() {
    let dir = tempfile::tempdir().unwrap();
    let seqs = store(dir.path());

    enqueue(dir.path(), "INBOX", &PimdirAction::Remove { seq: seqs[1] });

    let overlaid = PimdirReader::open(dir.path()).unwrap().with_pending();
    let listed: Vec<i64> = overlaid
        .list_items("INBOX", None, 10)
        .unwrap()
        .into_iter()
        .map(|item| item.seq)
        .collect();
    assert_eq!(listed, vec![seqs[0], seqs[2]]);
    assert!(overlaid.get_item("INBOX", seqs[1]).unwrap().is_none());
    assert_eq!(overlaid.count_items("INBOX").unwrap(), 2);

    let raw = PimdirReader::open(dir.path()).unwrap();
    assert_eq!(raw.count_items("INBOX").unwrap(), 3);
}

#[test]
fn a_staged_move_leaves_one_collection_and_enters_the_other() {
    let dir = tempfile::tempdir().unwrap();
    let seqs = store(dir.path());

    enqueue(
        dir.path(),
        "INBOX",
        &PimdirAction::Move {
            seq: seqs[2],
            to: ReplicaCollectionId("Archive".into()),
        },
    );

    let overlaid = PimdirReader::open(dir.path()).unwrap().with_pending();
    assert!(overlaid.get_item("INBOX", seqs[2]).unwrap().is_none());
    assert_eq!(overlaid.count_items("INBOX").unwrap(), 2);

    // NOTE: the moved item keeps its public id (spec §9.1), which is what
    // lets the overlay show it in the target without inventing one.
    let arrived = overlaid.get_item("Archive", seqs[2]).unwrap().unwrap();
    assert_eq!(arrived.seq, seqs[2]);
    assert_eq!(arrived.link_id, ReplicaLinkId("mid:c".into()));
    let listed: Vec<i64> = overlaid
        .list_items("Archive", None, 10)
        .unwrap()
        .into_iter()
        .map(|item| item.seq)
        .collect();
    assert_eq!(listed, vec![seqs[2]]);
    assert_eq!(overlaid.count_items("Archive").unwrap(), 1);
}

#[test]
fn a_staged_copy_enters_the_target_and_stays_in_the_source() {
    let dir = tempfile::tempdir().unwrap();
    let seqs = store(dir.path());

    enqueue(
        dir.path(),
        "INBOX",
        &PimdirAction::Copy {
            seq: seqs[0],
            to: ReplicaCollectionId("Archive".into()),
        },
    );

    let overlaid = PimdirReader::open(dir.path()).unwrap().with_pending();
    assert!(overlaid.get_item("INBOX", seqs[0]).unwrap().is_some());
    assert_eq!(overlaid.count_items("INBOX").unwrap(), 3);
    assert_eq!(overlaid.count_items("Archive").unwrap(), 1);
}

#[test]
fn a_sorted_page_stays_ordered_and_total_across_an_arrival() {
    let dir = tempfile::tempdir().unwrap();
    let seqs = store(dir.path());

    // the two outer items move into an archive that already holds none, so
    // the arrivals have to sort against each other, not merely append
    for seq in [seqs[2], seqs[0]] {
        enqueue(
            dir.path(),
            "INBOX",
            &PimdirAction::Move {
                seq,
                to: ReplicaCollectionId("Archive".into()),
            },
        );
    }

    let overlaid = PimdirReader::open(dir.path()).unwrap().with_pending();
    let descending: Vec<i64> = overlaid
        .list_items_page_desc("Archive", None, 10)
        .unwrap()
        .into_iter()
        .map(|item| item.seq)
        .collect();
    assert_eq!(descending, vec![seqs[2], seqs[0]]);

    // one item per page: the cursor from the first page must not repeat it
    let first = overlaid
        .list_items_page_desc("Archive", None, 1)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(first.seq, seqs[2]);
    let second = overlaid
        .list_items_page_desc("Archive", Some((&first.sort_key, first.seq)), 1)
        .unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].seq, seqs[0]);
}

#[test]
fn a_staged_create_is_counted_and_never_listed() {
    let dir = tempfile::tempdir().unwrap();
    let seqs = store(dir.path());

    enqueue(
        dir.path(),
        "INBOX",
        &PimdirAction::Add {
            link_id: Some(ReplicaLinkId("mid:draft".into())),
            flags: ReplicaFlags::from_iter(["\\Draft"]),
            object: None,
            meta: None,
            handle: None,
        },
    );

    let overlaid = PimdirReader::open(dir.path()).unwrap().with_pending();
    assert_eq!(overlaid.list_items("INBOX", None, 10).unwrap().len(), 3);
    assert_eq!(overlaid.count_items("INBOX").unwrap(), 3);
    assert_eq!(overlaid.count_pending_creates("INBOX").unwrap(), 1);
    assert_eq!(overlaid.count_pending_creates("Archive").unwrap(), 0);

    let creates = overlaid.pending_creates("INBOX").unwrap();
    assert_eq!(creates.len(), 1);
    assert_eq!(creates[0].created_at, NOW);
    assert_eq!(creates[0].producer, "test");
    let _ = seqs;
}

#[test]
fn a_parked_action_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let seqs = store(dir.path());

    enqueue(dir.path(), "INBOX", &PimdirAction::Remove { seq: seqs[1] });

    let owner = PimdirStore::open(dir.path()).unwrap();
    let id = owner.pending_actions("INBOX").unwrap()[0].id;
    owner.fail_action(id, Some("unappliable")).unwrap();
    drop(owner);

    // NOTE: a parked row says it will not be applied without an operator,
    // so reading it as pending would promise work nobody is doing.
    let overlaid = PimdirReader::open(dir.path()).unwrap().with_pending();
    assert_eq!(overlaid.count_items("INBOX").unwrap(), 3);
    assert!(overlaid.get_item("INBOX", seqs[1]).unwrap().is_some());
    assert_eq!(overlaid.pending_creates("INBOX").unwrap().len(), 0);
}

#[test]
fn the_scoped_cancel_takes_the_owner_role_only_for_the_call() {
    let dir = tempfile::tempdir().unwrap();
    let seqs = store(dir.path());

    enqueue(
        dir.path(),
        "INBOX",
        &PimdirAction::SetFlags {
            seq: seqs[0],
            flags: ReplicaFlags::from_iter(["\\Flagged"]),
        },
    );
    let id = PimdirReader::open(dir.path())
        .unwrap()
        .pending_actions("INBOX")
        .unwrap()[0]
        .id;

    // an owner in flight is refused at once, not waited out. Held through
    // a file description of its own, the way another process holds it:
    // one process owning a store twice is still one owner (spec §8).
    let held = File::open(dir.path().join("owner.lock")).unwrap();
    FileExt::try_lock(&held).unwrap();
    assert!(matches!(
        PimdirStore::cancel_action(dir.path(), id),
        Err(PimdirError::Owned(_))
    ));
    drop(held);

    assert!(PimdirStore::cancel_action(dir.path(), id).unwrap());
    // the role was released with the call, so the next owner opens fine
    assert!(PimdirStore::open(dir.path()).is_ok());

    let overlaid = PimdirReader::open(dir.path()).unwrap().with_pending();
    let item = overlaid.get_item("INBOX", seqs[0]).unwrap().unwrap();
    assert!(!item.flags.contains("\\Flagged"));
    assert!(!PimdirStore::cancel_action(dir.path(), id).unwrap());
}

#[test]
fn a_cancel_never_creates_the_store_it_cannot_find() {
    let dir = tempfile::tempdir().unwrap();

    assert!(matches!(
        PimdirStore::cancel_action(dir.path(), 1),
        Err(PimdirError::Uncreated)
    ));
    assert!(!dir.path().join("pimdir.db").exists());
}
