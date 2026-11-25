//! The reader role and its pending-action overlay (spec §8, §15.4): what
//! a frontend sees between staging a write and the owner applying it.

#![cfg(feature = "client")]

use std::fs::File;

use fs4::FileExt;
use io_pimdir::{
    change::PimdirWriteOp,
    collection::{PimdirCheckpoint, PimdirCollectionId},
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirSortKey, PimdirStatus,
    },
};
use io_pimdir::{
    client::reader::PimdirReader,
    client::{PimdirError, PimdirStore},
    codec::PimdirAction,
};

/// A hydrated, linked placement with a matching base, so it projects clean.
fn placement(collection: &str, handle: &str, link: &str, key: &str) -> PimdirPlacement {
    let flags = PimdirFlags::from_iter(["\\Seen"]);
    PimdirPlacement {
        sort_key: PimdirSortKey(key.into()),
        collection: PimdirCollectionId(collection.into()),
        handle: PimdirHandle(handle.into()),
        link_id: Some(PimdirLinkId(link.into())),
        object: Some(PimdirHash(
            "cafecafecafecafecafecafecafecafecafecafecafecafecafe".into(),
        )),
        level: PimdirLevel::Full,
        summary: None,
        flags: flags.clone(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags,
            revision: None,
            object: Some(PimdirHash(
                "cafecafecafecafecafecafecafecafecafecafecafecafecafe".into(),
            )),
        }),
        origin: None,
        server_object_id: None,
    }
}

/// An owner-created store holding `INBOX` (three items) and an empty
/// `Archive` left syncs, with the blob every item points at.
fn store(dir: &std::path::Path) -> Vec<i64> {
    let mut owner = PimdirStore::open(dir).unwrap().for_source("left").unwrap();
    owner.ensure_collection("INBOX", "message/rfc822").unwrap();
    owner
        .ensure_collection("Archive", "message/rfc822")
        .unwrap();
    owner
        .write(vec![
            PimdirWriteOp::StoreObject {
                object: PimdirObject {
                    hash: PimdirHash("cafecafecafecafecafecafecafecafecafecafecafecafecafe".into()),
                    size: 4,
                },
                body: Some(b"body".to_vec()),
            },
            PimdirWriteOp::UpsertPlacement(placement("INBOX", "1", "mid:a", "2026-01-01")),
            PimdirWriteOp::UpsertPlacement(placement("INBOX", "2", "mid:b", "2026-01-02")),
            PimdirWriteOp::UpsertPlacement(placement("INBOX", "3", "mid:c", "2026-01-03")),
            PimdirWriteOp::SetCheckpoint {
                collection: PimdirCollectionId("Archive".into()),
                checkpoint: PimdirCheckpoint(b"cp".to_vec()),
            },
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
    let mut producer = io_pimdir::client::producer::PimdirProducer::open(dir, "test").unwrap();
    producer.enqueue(collection, action, None).unwrap();
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
            flags: PimdirFlags::from_iter(["\\Flagged"]),
            base: None,
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
            flags: PimdirFlags::from_iter(["\\Flagged"]),
            base: None,
        },
    );
    enqueue(
        dir.path(),
        "INBOX",
        &PimdirAction::SetFlags {
            seq: seqs[0],
            flags: PimdirFlags::from_iter(["\\Answered"]),
            base: None,
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

    enqueue(
        dir.path(),
        "INBOX",
        &PimdirAction::Remove {
            seq: seqs[1],
            if_match: None,
            resolves: None,
        },
    );

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
            to: PimdirCollectionId("Archive".into()),
        },
    );

    let overlaid = PimdirReader::open(dir.path()).unwrap().with_pending();
    assert!(overlaid.get_item("INBOX", seqs[2]).unwrap().is_none());
    assert_eq!(overlaid.count_items("INBOX").unwrap(), 2);

    // NOTE: the moved item keeps its public id (spec §9.1), which is what
    // lets the overlay show it in the target without inventing one.
    let arrived = overlaid.get_item("Archive", seqs[2]).unwrap().unwrap();
    assert_eq!(arrived.seq, seqs[2]);
    assert_eq!(arrived.link_id, PimdirLinkId("mid:c".into()));
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
fn an_arrival_carries_the_flags_queued_on_it_before_its_move_or_copy() {
    let dir = tempfile::tempdir().unwrap();
    let seqs = store(dir.path());
    let flag = |seq: i64, flag: &str| {
        enqueue(
            dir.path(),
            "INBOX",
            &PimdirAction::SetFlags {
                seq,
                flags: PimdirFlags::from_iter([flag]),
                base: None,
            },
        );
    };

    flag(seqs[0], "\\Flagged");
    flag(seqs[1], "\\Flagged");
    for seq in [seqs[0], seqs[1]] {
        enqueue(
            dir.path(),
            "INBOX",
            &PimdirAction::Move {
                seq,
                to: PimdirCollectionId("Archive".into()),
            },
        );
    }
    flag(seqs[2], "\\Answered");
    enqueue(
        dir.path(),
        "INBOX",
        &PimdirAction::Copy {
            seq: seqs[2],
            to: PimdirCollectionId("Archive".into()),
        },
    );
    flag(seqs[2], "\\Draft");

    let overlaid = PimdirReader::open(dir.path()).unwrap().with_pending();
    let flags = |item: io_pimdir::client::reader::PimdirItem| (item.seq, item.flags);
    let set = |flag: &str| PimdirFlags::from_iter([flag]);
    let listed: Vec<(i64, PimdirFlags)> = overlaid
        .list_items("Archive", None, 10)
        .unwrap()
        .into_iter()
        .map(flags)
        .collect();
    assert_eq!(
        listed,
        vec![
            (seqs[0], set("\\Flagged")),
            (seqs[1], set("\\Flagged")),
            (seqs[2], set("\\Answered")),
        ],
        "a row queued after the copy stays in the source"
    );
    let got = overlaid.get_item("Archive", seqs[0]).unwrap().unwrap();
    assert_eq!(flags(got), (seqs[0], set("\\Flagged")));
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
            to: PimdirCollectionId("Archive".into()),
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
                to: PimdirCollectionId("Archive".into()),
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

    io_pimdir::client::producer::PimdirProducer::open(dir.path(), "test")
        .unwrap()
        .enqueue_with_body("INBOX", b"Subject: draft\r\n\r\nx\r\n", |hash| {
            PimdirAction::Add {
                link_id: Some(PimdirLinkId("mid:draft".into())),
                flags: PimdirFlags::from_iter(["\\Draft"]),
                object: Some(hash),
                supersedes: None,
            }
        })
        .unwrap();

    let overlaid = PimdirReader::open(dir.path()).unwrap().with_pending();
    assert_eq!(overlaid.list_items("INBOX", None, 10).unwrap().len(), 3);
    assert_eq!(overlaid.count_items("INBOX").unwrap(), 3);
    assert_eq!(overlaid.count_pending_creates("INBOX").unwrap(), 1);
    assert_eq!(overlaid.count_pending_creates("Archive").unwrap(), 0);

    let creates = overlaid.pending_creates("INBOX").unwrap();
    assert_eq!(creates.len(), 1);
    assert!(
        creates[0].created_at.ends_with('Z'),
        "SQLite stamps the row"
    );
    assert_eq!(creates[0].producer, "test");
    let _ = seqs;
}

#[test]
fn a_parked_action_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let seqs = store(dir.path());

    enqueue(
        dir.path(),
        "INBOX",
        &PimdirAction::Remove {
            seq: seqs[1],
            if_match: None,
            resolves: None,
        },
    );

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
            flags: PimdirFlags::from_iter(["\\Flagged"]),
            base: None,
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

#[test]
fn a_page_stays_full_across_a_staged_removal() {
    let dir = tempfile::tempdir().unwrap();
    let seqs = store(dir.path());

    // the first item of the collection goes, so a page of two asked from
    // the beginning has to reach past it rather than come back short
    enqueue(
        dir.path(),
        "INBOX",
        &PimdirAction::Remove {
            seq: seqs[0],
            if_match: None,
            resolves: None,
        },
    );

    let overlaid = PimdirReader::open(dir.path()).unwrap().with_pending();
    let page = overlaid.list_items("INBOX", None, 2).unwrap();
    assert_eq!(page.len(), 2);
    assert_eq!(
        page.iter().map(|item| item.seq).collect::<Vec<_>>(),
        vec![seqs[1], seqs[2]]
    );

    // NOTE: a caller paging until a short page is the ordinary contract,
    // and it is what a whole-collection scan is written against; a page cut
    // short in the middle would end that scan two items early.
    let last = page.last().unwrap();
    assert!(
        overlaid
            .list_items("INBOX", Some(&last.link_id.0), 2)
            .unwrap()
            .is_empty()
    );
}

/// The overlay shows the item as the drain will leave it (§15.3, §15.4): an
/// edit whose `if_match` names a body the item does not hold will park,
/// so it is not folded, while one naming the body an earlier queued edit
/// leaves is.
#[test]
fn an_edit_that_will_park_is_not_folded() {
    let dir = tempfile::tempdir().unwrap();
    let seqs = store(dir.path());
    let held = PimdirHash("cafecafecafecafecafecafecafecafecafecafecafecafecafe".into());

    let mut producer =
        io_pimdir::client::producer::PimdirProducer::open(dir.path(), "test").unwrap();
    let stale = producer.hash(b"a body read long ago");
    producer
        .enqueue_with_body("INBOX", b"Subject: lost\r\n\r\nlost", |hash| {
            PimdirAction::Update {
                seq: seqs[0],
                object: hash,
                if_match: Some(stale),
                resolves: None,
            }
        })
        .unwrap();
    let first = producer
        .enqueue_with_body("INBOX", b"Subject: one\r\n\r\none", |hash| {
            PimdirAction::Update {
                seq: seqs[1],
                object: hash,
                if_match: Some(held.clone()),
                resolves: None,
            }
        })
        .unwrap()
        .handle
        .id;
    let first = producer
        .pending_actions("INBOX")
        .unwrap()
        .into_iter()
        .find(|action| action.id == first)
        .unwrap();
    let PimdirAction::Update { object: one, .. } = first.action else {
        panic!("an update");
    };
    producer
        .enqueue_with_body("INBOX", b"Subject: two\r\n\r\ntwo", |hash| {
            PimdirAction::Update {
                seq: seqs[1],
                object: hash,
                if_match: Some(one),
                resolves: None,
            }
        })
        .unwrap();
    drop(producer);

    let reader = PimdirReader::open(dir.path()).unwrap().with_pending();
    let kept = reader.get_item("INBOX", seqs[0]).unwrap().unwrap();
    assert_eq!(
        kept.object,
        Some(held),
        "the edit that will park is not shown"
    );
    let chained = reader.get_item("INBOX", seqs[1]).unwrap().unwrap();
    let two = reader.hash(b"Subject: two\r\n\r\ntwo");
    assert_eq!(chained.object, Some(two), "an edit over a queued one is");
}

#[test]
fn a_collection_is_quiet_until_an_action_changes_what_its_read_shows() {
    let dir = tempfile::tempdir().unwrap();
    let seqs = store(dir.path());
    let quiet = |collection: &str| {
        let reader = PimdirReader::open(dir.path()).unwrap();
        reader.overlay_quiet(collection).unwrap()
    };
    assert!(quiet("INBOX"));
    assert!(quiet("Archive"));

    enqueue(
        dir.path(),
        "INBOX",
        &PimdirAction::Copy {
            seq: seqs[0],
            to: PimdirCollectionId("Archive".into()),
        },
    );
    assert!(quiet("INBOX"));
    assert!(!quiet("Archive"));

    enqueue(
        dir.path(),
        "INBOX",
        &PimdirAction::SetFlags {
            seq: seqs[1],
            flags: PimdirFlags::from_iter(["\\Flagged"]),
            base: None,
        },
    );
    assert!(!quiet("INBOX"));
}

/// A card in `collection` no source binds, written as a device keeps one.
fn local_card(dir: &std::path::Path, collection: &str, uid: &str, name: &str) -> i64 {
    let mut owner = PimdirStore::open(dir).unwrap();
    owner.ensure_collection(collection, "text/vcard").unwrap();
    let body = format!("BEGIN:VCARD\r\nVERSION:4.0\r\nUID:{uid}\r\nFN:{name}\r\nEND:VCARD\r\n");
    owner.put_item(collection, body.as_bytes()).unwrap().1
}

/// The full name a card item reads as.
fn full_name(item: &io_pimdir::client::reader::PimdirItem) -> String {
    match &item.summary {
        Some(io_pimdir::summary::PimdirSummary::Contact(card)) => card.full_name.clone(),
        other => panic!("no card summary: {other:?}"),
    }
}

/// STORAGE §15.3, §15.4: an item no source binds is the owner's to settle,
/// and the overlay folds its rows as the drain applies them. In a
/// collection no source syncs an update applies, its summary derived from
/// the new body, and a move parks; where a source syncs the collection a
/// set-flags and an update park. What the overlay shows is what the drain
/// leaves.
#[test]
fn an_item_no_source_binds_folds_as_the_owner_settles_it() {
    let dir = tempfile::tempdir().unwrap();
    let local = local_card(dir.path(), "Local", "a", "Ann Old");
    let synced = local_card(dir.path(), "Synced", "b", "Bob");
    PimdirStore::open(dir.path())
        .unwrap()
        .ensure_collection("Other", "text/vcard")
        .unwrap();
    PimdirStore::open(dir.path())
        .unwrap()
        .for_source("dav")
        .unwrap()
        .write(vec![PimdirWriteOp::SetCheckpoint {
            collection: PimdirCollectionId("Synced".into()),
            checkpoint: io_pimdir::collection::PimdirCheckpoint(b"c0".to_vec()),
        }])
        .unwrap();

    let mut producer =
        io_pimdir::client::producer::PimdirProducer::open(dir.path(), "test").unwrap();
    let renamed = b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:a\r\nFN:Ann New\r\nEND:VCARD\r\n";
    producer
        .enqueue_with_body("Local", renamed, |object| PimdirAction::Update {
            seq: local,
            object,
            if_match: None,
            resolves: None,
        })
        .unwrap();
    producer
        .enqueue(
            "Local",
            &PimdirAction::Move {
                seq: local,
                to: PimdirCollectionId("Other".into()),
            },
            None,
        )
        .unwrap();
    producer
        .enqueue(
            "Synced",
            &PimdirAction::SetFlags {
                seq: synced,
                flags: PimdirFlags::from_iter(["\\Flagged"]),
                base: None,
            },
            None,
        )
        .unwrap();
    drop(producer);

    let overlaid = PimdirReader::open(dir.path()).unwrap().with_pending();
    let card = overlaid
        .get_item("Local", local)
        .unwrap()
        .expect("a parked move leaves it");
    assert_eq!(full_name(&card), "Ann New", "the update's body summarized");
    assert!(overlaid.list_items("Other", None, 10).unwrap().is_empty());
    let card = overlaid.get_item("Synced", synced).unwrap().unwrap();
    assert!(!card.flags.contains("\\Flagged"), "the drain parks it");
    let shown = (
        overlaid
            .get_item("Local", local)
            .unwrap()
            .map(|card| card.object),
        overlaid
            .get_item("Synced", synced)
            .unwrap()
            .map(|card| card.flags),
    );
    drop(overlaid);

    let mut owner = PimdirStore::open(dir.path())
        .unwrap()
        .for_source("dav")
        .unwrap();
    let report = owner.drain().unwrap();
    assert_eq!((report.applied, report.parked), (1, 2), "{report:?}");
    let raw = PimdirReader::open(dir.path()).unwrap();
    let applied = (
        raw.get_item("Local", local)
            .unwrap()
            .map(|card| card.object),
        raw.get_item("Synced", synced)
            .unwrap()
            .map(|card| card.flags),
    );
    assert_eq!(shown, applied, "the overlay showed what the drain left");
}

/// STORAGE §15.2, §15.4: a move into a collection of no declared kind
/// parks, so the overlay leaves its item where it is.
#[test]
fn a_move_into_an_undeclared_collection_folds_into_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let seqs = store(dir.path());
    enqueue(
        dir.path(),
        "INBOX",
        &PimdirAction::Move {
            seq: seqs[0],
            to: PimdirCollectionId("Nowhere".into()),
        },
    );

    let overlaid = PimdirReader::open(dir.path()).unwrap().with_pending();
    assert!(overlaid.get_item("INBOX", seqs[0]).unwrap().is_some());
    drop(overlaid);

    let mut owner = PimdirStore::open(dir.path())
        .unwrap()
        .for_source("left")
        .unwrap();
    assert_eq!(owner.drain().unwrap().parked, 1);
}

/// STORAGE §15.3, §15.4: a move into a collection no source syncs parks,
/// so the overlay leaves its item where it is and brings nothing there.
#[test]
fn a_move_into_a_collection_no_source_syncs_folds_into_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let seqs = store(dir.path());
    PimdirStore::open(dir.path())
        .unwrap()
        .ensure_collection("Local", "message/rfc822")
        .unwrap();
    enqueue(
        dir.path(),
        "INBOX",
        &PimdirAction::Move {
            seq: seqs[0],
            to: PimdirCollectionId("Local".into()),
        },
    );

    let overlaid = PimdirReader::open(dir.path()).unwrap().with_pending();
    assert!(overlaid.get_item("INBOX", seqs[0]).unwrap().is_some());
    assert!(overlaid.get_item("Local", seqs[0]).unwrap().is_none());
    assert!(overlaid.list_items("Local", None, 10).unwrap().is_empty());
    drop(overlaid);

    let mut owner = PimdirStore::open(dir.path())
        .unwrap()
        .for_source("left")
        .unwrap();
    assert_eq!(owner.drain().unwrap().parked, 1);
}
