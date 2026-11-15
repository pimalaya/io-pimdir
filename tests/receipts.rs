//! Receipts (STORAGE §15.2, §15.4): a producer follows the row its
//! enqueue answered from pending to parked or applied, learning the seq
//! its `add` created once the row is gone, and a store whose owner
//! predates receipts gains the table on open. An intent its performer
//! acknowledges leaves one too (§15.5), a withdrawn row none.

#![cfg(feature = "client")]

use std::path::Path;

use io_pimdir::{
    change::{PimdirChangeKey, PimdirWriteOp},
    client::{
        PimdirStore,
        producer::{PimdirActionStatus, PimdirProducer},
        reader::PimdirReader,
    },
    codec::PimdirAction,
    collection::PimdirCollectionId,
    object::PimdirObject,
    placement::PimdirFlags,
};
use rusqlite::Connection;

const DRAFT: &str = "Message-ID: <draft@example.org>\r\nFrom: a@example.org\r\nTo: b@example.org\r\nSubject: hi\r\n\r\nhi\r\n";

/// An empty store with an INBOX and a Drafts, owned by source `local`.
fn store(dir: &Path) {
    let store = PimdirStore::open(dir).unwrap().for_source("local");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    store.ensure_collection("Drafts", "message/rfc822").unwrap();
}

/// Stages `body` through the producer and returns the `add` of it.
fn add(producer: &PimdirProducer, body: &str) -> (PimdirAction, PimdirObject) {
    let object = producer.stage_body(body.as_bytes()).unwrap();
    let action = PimdirAction::Add {
        link_id: None,
        flags: PimdirFlags::from_iter(["\\Draft"]),
        object: Some(object.hash.clone()),
        supersedes: None,
    };
    (action, object)
}

fn drain(dir: &Path) {
    let mut owner = PimdirStore::open(dir).unwrap().for_source("local");
    owner.drain().unwrap();
}

#[test]
fn an_applied_add_names_the_item_it_created() {
    let dir = tempfile::tempdir().unwrap();
    store(dir.path());

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let (action, object) = add(&producer, DRAFT);
    let id = producer
        .enqueue("Drafts", &action, Some(&object))
        .unwrap()
        .handle
        .id;
    assert_eq!(
        producer.action_status(id).unwrap(),
        PimdirActionStatus::Pending {
            collection: "Drafts".into(),
            kind: "add".into(),
            attempts: 0,
        }
    );

    drain(dir.path());

    let PimdirActionStatus::Applied {
        collection, seq, ..
    } = producer.action_status(id).unwrap()
    else {
        panic!("the drained add is applied");
    };
    assert_eq!(collection.as_deref(), Some("Drafts"));
    let reader = PimdirReader::open(dir.path()).unwrap();
    let items = reader.list_items("Drafts", None, 10).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(seq, Some(items[0].seq));
    assert_eq!(
        reader.action_status(id).unwrap(),
        producer.action_status(id).unwrap()
    );
}

#[test]
fn a_parked_row_reads_with_its_error_and_a_cancelled_one_nowhere() {
    let dir = tempfile::tempdir().unwrap();
    store(dir.path());

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let removal = producer
        .enqueue(
            "INBOX",
            &PimdirAction::Remove {
                seq: 41,
                if_match: None,
                resolves: None,
            },
            None,
        )
        .unwrap()
        .handle
        .id;
    let flags = producer
        .enqueue(
            "INBOX",
            &PimdirAction::SetFlags {
                seq: 41,
                flags: PimdirFlags::default(),
                base: None,
            },
            None,
        )
        .unwrap()
        .handle
        .id;
    let (action, object) = add(&producer, DRAFT);
    let cancelled = producer
        .enqueue("Drafts", &action, Some(&object))
        .unwrap()
        .handle
        .id;
    assert!(
        PimdirStore::open(dir.path())
            .unwrap()
            .drop_action(cancelled)
            .unwrap()
    );

    drain(dir.path());

    // a remove of nothing is a success with no item to name, a remove
    // naming the item it removed (§15.2)
    let PimdirActionStatus::Applied { seq: None, .. } = producer.action_status(removal).unwrap()
    else {
        panic!("a remove of an absent item is applied");
    };
    let PimdirActionStatus::Parked { error, kind, .. } = producer.action_status(flags).unwrap()
    else {
        panic!("a flag change on an unknown seq parks");
    };
    assert_eq!(kind, "set-flags");
    assert!(error.contains("41"), "{error}");
    assert_eq!(
        producer.action_status(cancelled).unwrap(),
        PimdirActionStatus::Unknown
    );
    assert_eq!(
        producer.action_status(9999).unwrap(),
        PimdirActionStatus::Unknown
    );
}

#[test]
fn a_duplicate_add_parks_and_names_no_item() {
    let dir = tempfile::tempdir().unwrap();
    store(dir.path());

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let (action, object) = add(&producer, DRAFT);
    let first = producer
        .enqueue("Drafts", &action, Some(&object))
        .unwrap()
        .handle
        .id;
    let second = producer
        .enqueue("Drafts", &action, Some(&object))
        .unwrap()
        .handle
        .id;

    drain(dir.path());

    let PimdirActionStatus::Applied { seq: Some(_), .. } = producer.action_status(first).unwrap()
    else {
        panic!("the first add is applied");
    };
    let PimdirActionStatus::Parked { .. } = producer.action_status(second).unwrap() else {
        panic!("the second add of the key parks");
    };
}

#[test]
fn an_old_receipt_is_pruned() {
    let dir = tempfile::tempdir().unwrap();
    store(dir.path());

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let (action, object) = add(&producer, DRAFT);
    let id = producer
        .enqueue("Drafts", &action, Some(&object))
        .unwrap()
        .handle
        .id;
    drain(dir.path());

    let db = Connection::open(dir.path().join("pimdir.db")).unwrap();
    db.execute(
        "UPDATE receipts SET applied_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-6 days')",
        [],
    )
    .unwrap();
    drain(dir.path());
    let PimdirActionStatus::Applied { .. } = producer.action_status(id).unwrap() else {
        panic!("a receipt younger than a week is kept");
    };

    db.execute(
        "UPDATE receipts SET applied_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-8 days')",
        [],
    )
    .unwrap();
    drain(dir.path());
    assert_eq!(
        producer.action_status(id).unwrap(),
        PimdirActionStatus::Unknown
    );
}

/// A `submit` intent of `body`, which the drain leaves to its performer.
fn submit(producer: &PimdirProducer, body: &str) -> (PimdirAction, PimdirObject) {
    let (_, object) = add(producer, body);
    let action = PimdirAction::Unknown {
        kind: "submit".into(),
        payload:
            "{\"v\":1,\"source\":\"smtp\",\"from\":\"a@example.org\",\"rcpts\":[\"b@example.org\"]}"
                .into(),
        object_hash: Some(object.hash.clone()),
    };
    (action, object)
}

fn refcount(dir: &Path, object: &PimdirObject) -> i64 {
    Connection::open(dir.join("pimdir.db"))
        .unwrap()
        .query_row(
            "SELECT refcount FROM objects WHERE hash = ?1",
            [&object.hash.0],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn an_acknowledged_intent_reads_applied_and_a_dropped_one_nowhere() {
    let dir = tempfile::tempdir().unwrap();
    store(dir.path());

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let (action, object) = submit(&producer, DRAFT);
    let sent = producer
        .enqueue("INBOX", &action, Some(&object))
        .unwrap()
        .handle
        .id;
    let withdrawn = producer
        .enqueue("INBOX", &action, Some(&object))
        .unwrap()
        .handle
        .id;
    assert_eq!(refcount(dir.path(), &object), 2);

    // the drain leaves intents to their performer
    drain(dir.path());
    let PimdirActionStatus::Pending { .. } = producer.action_status(sent).unwrap() else {
        panic!("the drain leaves a submit pending");
    };

    let mut owner = PimdirStore::open(dir.path()).unwrap().for_source("local");
    assert!(owner.acknowledge_action(sent, None).unwrap());
    assert!(!owner.acknowledge_action(sent, None).unwrap());
    assert!(owner.drop_action(withdrawn).unwrap());
    drop(owner);

    let PimdirActionStatus::Applied {
        collection, seq, ..
    } = producer.action_status(sent).unwrap()
    else {
        panic!("an acknowledged intent reads as applied");
    };
    assert_eq!(collection.as_deref(), Some("INBOX"));
    assert_eq!(seq, None);
    assert_eq!(
        producer.action_status(withdrawn).unwrap(),
        PimdirActionStatus::Unknown
    );
    assert_eq!(refcount(dir.path(), &object), 0);
}

#[test]
fn a_parked_intent_is_acknowledged_too() {
    let dir = tempfile::tempdir().unwrap();
    store(dir.path());

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let (action, object) = submit(&producer, DRAFT);
    let id = producer
        .enqueue("INBOX", &action, Some(&object))
        .unwrap()
        .handle
        .id;

    let mut owner = PimdirStore::open(dir.path()).unwrap().for_source("local");
    owner.fail_action(id, Some("refused")).unwrap();
    let PimdirActionStatus::Parked { .. } = producer.action_status(id).unwrap() else {
        panic!("the failure parks the intent");
    };
    assert!(owner.acknowledge_action(id, Some(7)).unwrap());
    let PimdirActionStatus::Applied { seq: Some(7), .. } = producer.action_status(id).unwrap()
    else {
        panic!("a parked intent performed after all reads as applied, naming what it left");
    };
}

/// A receipt outlives the collection its row was queued on (STORAGE
/// §15.4): the collection deleted, the action still reads applied, naming
/// no collection, never withdrawn.
#[test]
fn a_receipt_outlives_its_collection_and_names_none() {
    let dir = tempfile::tempdir().unwrap();
    store(dir.path());

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let (action, object) = add(&producer, DRAFT);
    let id = producer
        .enqueue("Drafts", &action, Some(&object))
        .unwrap()
        .handle
        .id;
    drain(dir.path());
    let PimdirActionStatus::Applied { seq: created, .. } = producer.action_status(id).unwrap()
    else {
        panic!("the drained add is applied");
    };
    assert!(created.is_some());

    let mut owner = PimdirStore::open(dir.path()).unwrap().for_source("local");
    assert!(owner.delete_collection("Drafts").unwrap());
    drop(owner);

    let PimdirActionStatus::Applied {
        collection, seq, ..
    } = producer.action_status(id).unwrap()
    else {
        panic!("the receipt went with its collection");
    };
    assert_eq!((collection, seq), (None, created));
}

#[test]
fn a_replaced_intent_reads_applied_and_its_change_is_followed_apart() {
    let dir = tempfile::tempdir().unwrap();
    store(dir.path());

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let (action, object) = submit(&producer, DRAFT);
    let id = producer
        .enqueue("INBOX", &action, Some(&object))
        .unwrap()
        .handle
        .id;

    let mut owner = PimdirStore::open(dir.path()).unwrap().for_source("local");
    let copy = PimdirAction::Add {
        link_id: None,
        flags: PimdirFlags::from_iter(["\\Seen"]),
        object: Some(object.hash.clone()),
        supersedes: None,
    };
    assert!(owner.replace_action(id, "owner", "Drafts", &copy).unwrap());
    assert!(!owner.replace_action(id, "owner", "Drafts", &copy).unwrap());
    let change = owner.list_pending_actions().unwrap()[0].id;
    drop(owner);

    let PimdirActionStatus::Applied {
        collection, seq, ..
    } = producer.action_status(id).unwrap()
    else {
        panic!("a replaced intent reads as applied");
    };
    assert_eq!((collection.as_deref(), seq), (Some("INBOX"), None));

    drain(dir.path());
    let PimdirActionStatus::Applied { seq: Some(_), .. } = producer.action_status(change).unwrap()
    else {
        panic!("the copy is followed by its own id");
    };
}

/// Drains one action of `producer` on `collection` and answers its status.
fn applied(
    dir: &Path,
    producer: &mut PimdirProducer,
    collection: &str,
    action: PimdirAction,
) -> PimdirActionStatus {
    let id = producer
        .enqueue(collection, &action, None)
        .unwrap()
        .handle
        .id;
    drain(dir);
    producer.action_status(id).unwrap()
}

#[test]
fn every_receipt_names_the_item_its_row_acted_on() {
    let dir = tempfile::tempdir().unwrap();
    store(dir.path());

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let (action, object) = add(&producer, DRAFT);
    producer.enqueue("Drafts", &action, Some(&object)).unwrap();
    drain(dir.path());
    let reader = PimdirReader::open(dir.path()).unwrap();
    let seq = reader.list_items("Drafts", None, 10).unwrap()[0].seq;

    let flagged = PimdirAction::SetFlags {
        seq,
        flags: PimdirFlags::from_iter(["\\Draft", "\\Seen"]),
        base: None,
    };
    let PimdirActionStatus::Applied { seq: named, .. } =
        applied(dir.path(), &mut producer, "Drafts", flagged)
    else {
        panic!("the flag change is applied");
    };
    assert_eq!(named, Some(seq), "a set-flags names the item it changed");

    let moved = PimdirAction::Move {
        seq,
        to: PimdirCollectionId::from("INBOX"),
    };
    let PimdirActionStatus::Applied { seq: named, .. } =
        applied(dir.path(), &mut producer, "Drafts", moved)
    else {
        panic!("the move is applied");
    };
    let landed = reader.list_items("INBOX", None, 10).unwrap();
    assert_eq!(landed.len(), 1);
    assert_eq!(
        named,
        Some(landed[0].seq),
        "a move names the item it left in its target"
    );
}

#[test]
fn a_refusal_shows_on_the_receipt_and_keeps_it_until_cleared() {
    let dir = tempfile::tempdir().unwrap();
    store(dir.path());

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let (action, object) = add(&producer, DRAFT);
    let id = producer
        .enqueue("Drafts", &action, Some(&object))
        .unwrap()
        .handle
        .id;
    drain(dir.path());
    let reader = PimdirReader::open(dir.path()).unwrap();
    let item = reader.list_items("Drafts", None, 10).unwrap().remove(0);

    // NOTE: what the engine writes on a Refused outcome (SYNC §5), a
    // server's text carrying a credential the store never keeps (§18).
    let mut owner = PimdirStore::open(dir.path()).unwrap().for_source("local");
    owner
        .write(vec![PimdirWriteOp::RefusePlacement {
            collection: PimdirCollectionId::from("Drafts"),
            handle: item.link_id.provisional(),
            key: PimdirChangeKey::from("0123456789abcdef"),
            reason: String::from("appends go to Drafts only: https://me:hunter2@graph.example/x"),
        }])
        .unwrap();

    let PimdirActionStatus::Applied { refusals, seq, .. } = producer.action_status(id).unwrap()
    else {
        panic!("the refused add is still applied");
    };
    assert_eq!(seq, Some(item.seq));
    assert_eq!(refusals.len(), 1, "{refusals:?}");
    assert_eq!(
        (refusals[0].collection.as_str(), refusals[0].source.as_str()),
        ("Drafts", "local")
    );
    assert!(refusals[0].reason.starts_with("appends go to Drafts only"));
    assert!(
        !refusals[0].reason.contains("hunter2"),
        "{}",
        refusals[0].reason
    );
    assert_eq!(reader.receipt_refusals(id).unwrap(), refusals);
    let listed = reader.refusals("Drafts").unwrap();
    assert_eq!(
        (listed[0].seq, listed[0].key.as_str()),
        (item.seq, "0123456789abcdef")
    );

    let db = Connection::open(dir.path().join("pimdir.db")).unwrap();
    let age = || {
        db.execute(
            "UPDATE receipts SET applied_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-8 days')",
            [],
        )
        .unwrap()
    };
    age();
    drain(dir.path());
    let PimdirActionStatus::Applied { .. } = producer.action_status(id).unwrap() else {
        panic!("a receipt is kept while a refusal stands on its item");
    };

    owner
        .write(vec![PimdirWriteOp::ClearRefusal {
            collection: PimdirCollectionId::from("Drafts"),
            handle: item.link_id.provisional(),
        }])
        .unwrap();
    assert_eq!(reader.refusals("Drafts").unwrap(), Vec::new());
    age();
    drain(dir.path());
    assert_eq!(
        producer.action_status(id).unwrap(),
        PimdirActionStatus::Unknown
    );
}
