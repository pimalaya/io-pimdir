//! Receipts (STORAGE §15.2, §15.4): a producer follows the row its
//! enqueue answered from pending to parked or applied, learning the seq
//! its `add` created once the row is gone, and a store whose owner
//! predates receipts gains the table on open.

use std::{io::Write, path::Path};

use io_pimdir::{
    client::{
        PimdirStore,
        producer::{PimdirActionStatus, PimdirProducer},
        reader::PimdirReader,
    },
    codec::PimdirAction,
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

/// Writes `body` through the producer's blobs and returns the `add` of it.
fn add(producer: &PimdirProducer, body: &str) -> (PimdirAction, PimdirObject) {
    let blobs = producer.blobs();
    let hash = blobs.hash(body.as_bytes());
    let mut writer = blobs.writer().unwrap();
    writer.write_all(body.as_bytes()).unwrap();
    writer.commit(&hash).unwrap();
    let action = PimdirAction::Add {
        link_id: None,
        flags: PimdirFlags::from_iter(["\\Draft"]),
        object: Some(hash.clone()),
    };
    (
        action,
        PimdirObject {
            hash,
            size: body.len(),
        },
    )
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
    let id = producer.enqueue("Drafts", &action, Some(&object)).unwrap();
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
    assert_eq!(collection, "Drafts");
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
        .enqueue("INBOX", &PimdirAction::Remove { seq: 41 }, None)
        .unwrap();
    let flags = producer
        .enqueue(
            "INBOX",
            &PimdirAction::SetFlags {
                seq: 41,
                flags: PimdirFlags::default(),
            },
            None,
        )
        .unwrap();
    let (action, object) = add(&producer, DRAFT);
    let cancelled = producer.enqueue("Drafts", &action, Some(&object)).unwrap();
    assert!(
        PimdirStore::open(dir.path())
            .unwrap()
            .drop_action(cancelled)
            .unwrap()
    );

    drain(dir.path());

    // a remove of nothing is a success with no item to name
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
    let first = producer.enqueue("Drafts", &action, Some(&object)).unwrap();
    let second = producer.enqueue("Drafts", &action, Some(&object)).unwrap();

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
    let id = producer.enqueue("Drafts", &action, Some(&object)).unwrap();
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

#[test]
fn a_store_from_an_older_owner_gains_the_receipts_on_open() {
    let dir = tempfile::tempdir().unwrap();
    store(dir.path());
    Connection::open(dir.path().join("pimdir.db"))
        .unwrap()
        .execute_batch("DROP TABLE receipts;")
        .unwrap();

    // a producer reads the missing table as no receipt kept
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let (action, object) = add(&producer, DRAFT);
    let id = producer.enqueue("Drafts", &action, Some(&object)).unwrap();
    Connection::open(dir.path().join("pimdir.db"))
        .unwrap()
        .execute("DELETE FROM queue WHERE id = ?1", [id])
        .unwrap();
    assert_eq!(
        producer.action_status(id).unwrap(),
        PimdirActionStatus::Unknown
    );

    // the owner reconciles it on open, and the next add is followed
    let id = producer.enqueue("Drafts", &action, Some(&object)).unwrap();
    drain(dir.path());
    let PimdirActionStatus::Applied { seq: Some(_), .. } = producer.action_status(id).unwrap()
    else {
        panic!("the reconciled store keeps receipts");
    };
}
