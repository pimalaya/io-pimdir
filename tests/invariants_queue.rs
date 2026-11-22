//! Pending actions across a collection rename (STORAGE §14, §15.3),
//! ported from pimdir's retired checks/invariants.sh: a move, and an add
//! superseding an item, follow their target into its new id, a plain add
//! naming none.

#![cfg(feature = "client")]

mod invariants_common;

use std::path::Path;

use io_pimdir::{
    change::PimdirWriteOp,
    client::producer::PimdirProducer,
    codec::{PimdirAction, PimdirSupersedes},
    collection::PimdirCollectionId,
    load::PimdirLoadSelection,
    placement::{PimdirFlags, PimdirHandle, PimdirLinkId, PimdirStatus},
};

use invariants_common::*;

const AUG: &str = "Sat, 1 Aug 2026 10:00:00 +0000";

/// Every queue row's action and target, in append order.
fn targets(dir: &Path) -> Vec<(String, Option<String>)> {
    let conn = peek(dir);
    let mut stmt = conn
        .prepare("SELECT action, target_collection FROM queue ORDER BY id")
        .unwrap();
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// L558, L1722: a rename follows into a pending move's target and into a
/// superseding add's, and leaves a plain add naming none.
#[test]
fn l558_l1722_a_rename_follows_into_pending_targets() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    store.ensure_collection("Sent", MAIL).unwrap();
    let a = message("a@x", "A", AUG, Some("alice@x"));
    let b = message("b@x", "B", AUG, Some("alice@x"));
    store
        .write(vec![
            body(&store, &a),
            body(&store, &b),
            PimdirWriteOp::UpsertPlacement(held(&store, "INBOX", "1", &a)),
            PimdirWriteOp::UpsertPlacement(held(&store, "INBOX", "2", &b)),
        ])
        .unwrap();
    let seq_a = store.seq_for_link("INBOX", "a@x").unwrap().unwrap();
    let seq_b = store.seq_for_link("INBOX", "b@x").unwrap().unwrap();

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::Move {
                seq: seq_a,
                to: PimdirCollectionId("Archive".into()),
            },
            None,
        )
        .unwrap();
    let fresh = message("c@x", "C", AUG, Some("alice@x"));
    let staged = producer.stage_body(&fresh).unwrap();
    producer
        .enqueue_add(
            "Sent",
            &PimdirLinkId("c@x".into()),
            PimdirFlags::default(),
            Some(&staged),
            Some(PimdirSupersedes {
                collection: PimdirCollectionId("INBOX".into()),
                seq: seq_b,
            }),
        )
        .unwrap();
    producer
        .enqueue_add(
            "Sent",
            &PimdirLinkId("d@x".into()),
            PimdirFlags::default(),
            Some(&staged),
            None,
        )
        .unwrap();
    drop(producer);
    let before = targets(dir.path());
    assert_eq!(
        before,
        [
            ("move".into(), Some("Archive".into())),
            ("add".into(), Some("INBOX".into())),
            ("add".into(), None),
        ]
    );

    store.rename_collection("Archive", "Archive-2026").unwrap();
    store.rename_collection("INBOX", "Inbox").unwrap();

    assert_eq!(
        targets(dir.path()),
        [
            ("move".into(), Some("Archive-2026".into())),
            ("add".into(), Some("Inbox".into())),
            ("add".into(), None),
        ],
        "L558: the move follows; L1722: the superseding add follows, a plain one names none"
    );
}

fn created(collection: &str, handle: &str, link: &str) -> PimdirWriteOp {
    let mut placement = bare(collection, handle, link);
    placement.status = PimdirStatus::Created;
    placement.base = None;
    PimdirWriteOp::UpsertPlacement(placement)
}

fn delivered(collection: &str, handle: &str) -> PimdirWriteOp {
    PimdirWriteOp::DeliverPlacement {
        collection: PimdirCollectionId(collection.into()),
        handle: PimdirHandle(handle.into()),
    }
}

/// The origin or destination `load` gives the placement under `handle`.
fn origin_of(
    store: &io_pimdir::client::PimdirSourceStore,
    collection: &str,
    handle: &str,
) -> Option<(String, String)> {
    store
        .load(
            &PimdirCollectionId(collection.into()),
            &PimdirLoadSelection::All,
        )
        .unwrap()
        .placements
        .into_iter()
        .find(|placement| placement.handle.0 == handle)
        .unwrap_or_else(|| panic!("{collection} lists {handle:?}"))
        .origin
        .map(|origin| (origin.collection.0, origin.handle.0))
}

/// L2326: a delivered create is no origin a copy comes from; a member is.
#[test]
fn l2326_a_delivered_create_is_no_origin() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    store
        .write(vec![
            PimdirWriteOp::UpsertPlacement(bare("Archive", "10", "kept")),
            created("Archive", "\u{1}moved", "moved"),
        ])
        .unwrap();
    store
        .write(vec![delivered("Archive", "\u{1}moved")])
        .unwrap();
    store
        .write(vec![
            created("INBOX", "\u{1}k", "kept"),
            created("INBOX", "\u{1}m", "moved"),
        ])
        .unwrap();

    assert_eq!(
        origin_of(&store, "INBOX", "\u{1}k"),
        Some(("Archive".into(), "10".into())),
        "a member is the origin of a copy"
    );
    assert_eq!(origin_of(&store, "INBOX", "\u{1}m"), None);
}

/// L2329: a delivered create stays the destination a tombstone of its
/// identity relocates into.
#[test]
fn l2329_a_delivered_create_stays_the_destination() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(bare(
            "INBOX", "11", "moved",
        ))])
        .unwrap();
    store
        .write(vec![created("Archive", "\u{1}moved", "moved")])
        .unwrap();
    store
        .write(vec![delivered("Archive", "\u{1}moved")])
        .unwrap();
    let mut tombstone = bare("INBOX", "11", "moved");
    tombstone.status = PimdirStatus::Tombstone;
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(tombstone)])
        .unwrap();

    assert_eq!(
        origin_of(&store, "INBOX", "11"),
        Some(("Archive".into(), "\u{1}moved".into()))
    );
}
