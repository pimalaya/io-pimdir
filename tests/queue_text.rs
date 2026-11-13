//! The queue rules draft 10 put into text (pimdir STORAGE §13, §15.1,
//! §15.2, §15.4, §15.5): a parked row retried on request, an intent
//! whose outcome is unknown, several actions enqueued in one transaction,
//! and a run of actions applied in one, each under its savepoint.

#![cfg(feature = "client")]

use std::path::Path;

use io_pimdir::{
    change::PimdirWriteOp,
    client::{
        PimdirDuplicate, PimdirError, PimdirSourceStore, PimdirStore,
        producer::{PimdirActionStatus, PimdirEnqueue, PimdirProducer},
    },
    codec::{PimdirAction, UNCONFIRMED},
    collection::PimdirCollectionId,
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
    sql,
};

const SEEDED: &str = "cafebabecafebabecafebabecafebabecafebabecafebabecafe";

/// A store whose INBOX holds `items` clean messages, handles `1` to
/// `items`, answering their public ids in handle order.
fn seeded(dir: &Path, items: usize) -> (PimdirSourceStore, Vec<i64>) {
    let mut store = PimdirStore::open(dir).unwrap().for_source("local");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    let mut ops = vec![PimdirWriteOp::StoreObject {
        object: PimdirObject {
            hash: PimdirHash(SEEDED.into()),
            size: 3,
        },
        body: Some(b"abc".to_vec()),
    }];
    for handle in 1..=items {
        ops.push(PimdirWriteOp::UpsertPlacement(PimdirPlacement {
            sort_key: Default::default(),
            collection: PimdirCollectionId("INBOX".into()),
            handle: PimdirHandle(format!("{handle:04}")),
            link_id: Some(PimdirLinkId(format!("mid:{handle:04}"))),
            object: Some(PimdirHash(SEEDED.into())),
            level: PimdirLevel::Full,
            summary: None,
            flags: PimdirFlags::default(),
            status: PimdirStatus::Clean,
            conflict_revision: None,
            conflict_object: None,
            base: Some(PimdirBase {
                flags: PimdirFlags::default(),
                revision: None,
                object: Some(PimdirHash(SEEDED.into())),
            }),
            origin: None,
        }));
    }
    store.write(ops).unwrap();

    let mut seqs: Vec<(String, i64)> = store
        .list_items("INBOX", None, items.max(1))
        .unwrap()
        .into_iter()
        .map(|item| (item.link_id.0, item.seq))
        .collect();
    seqs.sort();
    (store, seqs.into_iter().map(|(_, seq)| seq).collect())
}

fn flag(seq: i64, flag: &str) -> PimdirAction {
    PimdirAction::SetFlags {
        seq,
        flags: PimdirFlags::from_iter([flag]),
        base: None,
    }
}

/// Appends a row past every role of the crate, as a later program or a
/// careless one would: an intent this owner performs nothing of.
fn enqueue_raw(dir: &Path, action: &str, payload: &str) -> i64 {
    let conn = rusqlite::Connection::open(dir.join("pimdir.db")).unwrap();
    conn.execute(
        sql::ENQUEUE_ACTION,
        rusqlite::named_params! {
            ":producer": "frontend",
            ":collection": "INBOX",
            ":action": action,
            ":payload": payload,
            ":target_collection": None::<String>,
            ":object_hash": None::<String>,
        },
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// The row `id` as the queue holds it: its attempts and its error.
fn row(dir: &Path, id: i64) -> Option<(i64, Option<String>)> {
    let conn = rusqlite::Connection::open(dir.join("pimdir.db")).unwrap();
    conn.query_row(
        "SELECT attempts, error FROM queue WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .ok()
}

/// `retry_action` puts a parked row back among the pending ones (STORAGE
/// §15.5): its error cleared, its id and its attempts kept, so the next
/// drain meets it in its place. It answers nothing for a row pending or
/// gone, and refuses a handle of another incarnation.
#[test]
fn a_parked_row_is_retried_on_request_keeping_its_id_and_attempts() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seqs) = seeded(dir.path(), 1);
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();

    // an edit of an item that is not there yet parks
    let missing = producer
        .enqueue("INBOX", &flag(seqs[0] + 100, "\\Seen"), None)
        .unwrap()
        .handle;
    let pending = producer
        .enqueue("INBOX", &flag(seqs[0], "\\Seen"), None)
        .unwrap()
        .handle;
    assert!(!store.retry_action(&pending).unwrap(), "a pending row");
    assert_eq!(store.drain().unwrap().parked, 1);
    assert_eq!(row(dir.path(), missing.id).unwrap().0, 1);
    assert!(!store.retry_action(&pending).unwrap(), "a row gone");

    assert!(store.retry_action(&missing).unwrap());
    assert_eq!(
        row(dir.path(), missing.id),
        Some((1, None)),
        "the error cleared, the attempts kept"
    );
    assert!(matches!(
        producer.action_status(&missing).unwrap(),
        PimdirActionStatus::Pending { attempts: 1, .. }
    ));
    assert!(!store.retry_action(&missing).unwrap(), "pending again");

    // retried, it meets the same store and parks again
    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (0, 1));
    assert_eq!(row(dir.path(), missing.id).unwrap().0, 2);

    store.raise_incarnation(None).unwrap();
    let refused = store.retry_action(&missing).unwrap_err();
    assert_eq!(refused.code(), "store-reset");
    assert!(row(dir.path(), missing.id).unwrap().1.is_some(), "kept");
    assert!(store.retry_action(missing.id).unwrap(), "a bare id");
}

/// An intent whose outcome is unknown is parked under the reserved
/// prefix (STORAGE §15.2): the prefix is the crate's, the detail the
/// caller's; the state reads `Unconfirmed`; no drain retries it; and it
/// leaves by its performer's acknowledgement, with its receipt, by a
/// retry on request, or by a cancellation.
#[test]
fn an_unconfirmed_intent_is_retried_by_no_drain_and_leaves_three_ways() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, _) = seeded(dir.path(), 1);
    let producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let submit = r#"{"v":1,"source":"smtp","from":"a@x.org","rcpts":["b@y.org"]}"#;
    let ids: Vec<i64> = (0..3)
        .map(|_| enqueue_raw(dir.path(), "x-fax-send", submit))
        .collect();

    for id in &ids {
        assert!(
            store
                .unconfirm_action(*id, "connection dropped after DATA, password=hunter2")
                .unwrap()
        );
    }
    assert!(!store.unconfirm_action(9_999, "no row").unwrap());

    let (attempts, error) = row(dir.path(), ids[0]).unwrap();
    let error = error.unwrap();
    assert_eq!(attempts, 1, "the attempt counted");
    assert!(error.starts_with(UNCONFIRMED), "{error}");
    assert!(!error.contains("hunter2"), "scrubbed as a failure is");
    let PimdirActionStatus::Unconfirmed { detail, kind, .. } =
        producer.action_status(ids[0]).unwrap()
    else {
        panic!("the state is read from the prefix");
    };
    assert_eq!(kind, "x-fax-send");
    assert!(detail.starts_with("connection dropped after DATA"));
    let parked = store.parked_actions().unwrap();
    assert_eq!(parked.len(), 3);
    assert!(parked.iter().all(|row| row.unconfirmed().is_some()));

    // no drain lists it: nothing performs it again of its own accord
    for _ in 0..3 {
        let report = store.drain().unwrap();
        assert_eq!(
            (
                report.applied,
                report.parked,
                report.skipped,
                report.retried
            ),
            (0, 0, 0, 0)
        );
    }
    assert_eq!(row(dir.path(), ids[0]).unwrap().0, 1, "untouched");
    assert!(store.list_pending_actions().unwrap().is_empty());

    // confirmed by its performer: acknowledged, with its receipt
    assert!(store.acknowledge_action(ids[0], None).unwrap());
    assert!(matches!(
        producer.action_status(ids[0]).unwrap(),
        PimdirActionStatus::Applied { .. }
    ));

    // retried on a person's request: pending again, in its place
    assert!(store.retry_action(ids[1]).unwrap());
    assert!(matches!(
        producer.action_status(ids[1]).unwrap(),
        PimdirActionStatus::Pending { attempts: 1, .. }
    ));
    assert_eq!(store.list_pending_actions().unwrap()[0].id, ids[1]);

    // withdrawn
    assert!(store.drop_action(ids[2]).unwrap());
    assert_eq!(
        producer.action_status(ids[2]).unwrap(),
        PimdirActionStatus::Unknown
    );
}

/// No caller's free text writes the reserved prefix, and none loses it
/// (STORAGE §13): a failure whose text begins with it is recorded behind
/// `failed: ` and reads parked, and an action the owner applies itself
/// is never marked unconfirmed.
#[test]
fn a_failure_text_never_forges_the_reserved_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seqs) = seeded(dir.path(), 1);
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let intent = enqueue_raw(dir.path(), "x-fax-send", r#"{"v":1}"#);

    store
        .fail_action(intent, Some("unconfirmed: 451 try again later"))
        .unwrap();
    let PimdirActionStatus::Parked { error, .. } = producer.action_status(intent).unwrap() else {
        panic!("a failure reads parked, whatever its text");
    };
    assert_eq!(error, "failed: unconfirmed: 451 try again later");
    assert!(store.parked_actions().unwrap()[0].unconfirmed().is_none());

    // the same row, its outcome now unknown: the prefix is written here
    assert!(store.unconfirm_action(intent, "unconfirmed: x").unwrap());
    let PimdirActionStatus::Unconfirmed { detail, .. } = producer.action_status(intent).unwrap()
    else {
        panic!("unconfirmed");
    };
    assert_eq!(detail, "unconfirmed: x", "the caller's text, whole");

    // an action the owner applies commits or does not
    let applied = producer
        .enqueue("INBOX", &flag(seqs[0], "\\Seen"), None)
        .unwrap()
        .handle;
    let refused = store.unconfirm_action(applied.id, "?").unwrap_err();
    assert_eq!(refused.code(), "action-not-intent");
    assert_eq!(row(dir.path(), applied.id), Some((0, None)), "untouched");
}

/// Several actions enqueued in one transaction are appended all or none,
/// in the order given (STORAGE §15.1): one refused leaves the queue as
/// it was and no body in the tree, and a batch that passes answers each
/// row's handle and the partial supports its gate computed.
#[test]
fn a_batch_is_appended_whole_in_order_or_not_at_all() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seqs) = seeded(dir.path(), 2);
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let body: &[u8] = b"Message-ID: <new@x>\r\nSubject: new\r\n\r\nbody";
    let hash = producer.hash(body);
    let add = |link: &str| PimdirAction::Add {
        link_id: Some(PimdirLinkId(link.into())),
        flags: PimdirFlags::default(),
        object: Some(hash.clone()),
        supersedes: None,
    };

    // the third action names a link id a live item holds: nothing lands
    let refused = producer
        .enqueue_batch(&[
            PimdirEnqueue::new("INBOX", flag(seqs[0], "\\Seen")),
            PimdirEnqueue::new("INBOX", add("mid:new")).body(body),
            PimdirEnqueue::new("INBOX", add("mid:0001")).body(body),
        ])
        .unwrap_err();
    assert!(matches!(
        refused,
        PimdirError::Duplicate {
            existing: PimdirDuplicate::Live(_),
            ..
        }
    ));
    assert!(store.list_pending_actions().unwrap().is_empty());
    assert!(
        store.blobs().get(&hash).unwrap().is_none(),
        "no body staged"
    );
    assert!(!store.indexed_hashes().unwrap().contains(&hash.0));
    assert!(store.collect_garbage().is_ok(), "the window is closed");

    // two adds of one key in a batch: the second meets the first's row
    let refused = producer
        .enqueue_batch(&[
            PimdirEnqueue::new("INBOX", add("mid:new")).body(body),
            PimdirEnqueue::new("INBOX", add("mid:new")),
        ])
        .unwrap_err();
    assert!(matches!(
        refused,
        PimdirError::Duplicate {
            existing: PimdirDuplicate::Pending(_),
            ..
        }
    ));
    assert!(store.list_pending_actions().unwrap().is_empty());

    // bytes that are not the body the action names refuse the batch
    assert!(
        producer
            .enqueue_batch(&[PimdirEnqueue::new("INBOX", add("mid:new")).body(b"other")])
            .is_err()
    );

    let enqueued = producer
        .enqueue_batch(&[
            PimdirEnqueue::new("INBOX", flag(seqs[0], "\\Seen")),
            PimdirEnqueue::new("INBOX", add("mid:new")).body(body),
            PimdirEnqueue::new("INBOX", flag(seqs[1], "\\Flagged")),
        ])
        .unwrap();
    assert_eq!(enqueued.len(), 3);
    let ids: Vec<i64> = enqueued.iter().map(|row| row.handle.id).collect();
    assert_eq!(ids, vec![ids[0], ids[0] + 1, ids[0] + 2], "one run of ids");
    assert!(enqueued.iter().all(|row| row.partial.is_empty()));
    assert!(enqueued.iter().all(|row| row.handle.store.is_some()));
    let pending = store.list_pending_actions().unwrap();
    assert_eq!(
        pending.iter().map(|row| row.id).collect::<Vec<_>>(),
        ids,
        "in the order enqueued"
    );
    assert_eq!(store.blobs().get(&hash).unwrap().as_deref(), Some(body));
    assert!(store.collect_garbage().is_ok(), "the window is closed");
    assert!(store.blobs().get(&hash).unwrap().is_some(), "pinned");

    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (3, 0));
    assert_eq!(store.list_items("INBOX", None, 10).unwrap().len(), 3);
}

/// A run of actions is one transaction, a savepoint per action (STORAGE
/// §15.2): an action failing in the middle is rolled back alone, its row
/// restored and parked or bumped, and the actions around it apply, each
/// with its receipt, across more rows than one run holds.
#[test]
fn a_failing_action_in_a_run_parks_alone_and_its_neighbours_apply() {
    let dir = tempfile::tempdir().unwrap();
    let items = PimdirSourceStore::RUN * 2 + 7;
    let (mut store, seqs) = seeded(dir.path(), items);
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();

    // every third row fails: an unknown seq parks, a body gone since
    // its enqueue is the environment's and is retried
    let gone: &[u8] = b"Message-ID: <gone@x>\r\nSubject: gone\r\n\r\nbody";
    let object = producer.stage_body(gone).unwrap();
    let mut batch = Vec::new();
    for (at, seq) in seqs.iter().enumerate() {
        batch.push(match at % 3 {
            0 => PimdirEnqueue::new("INBOX", flag(*seq, "\\Seen")),
            1 => PimdirEnqueue::new("INBOX", flag(seq + 10_000, "\\Seen")),
            _ => PimdirEnqueue::new(
                "INBOX",
                PimdirAction::Update {
                    seq: *seq,
                    object: object.hash.clone(),
                    if_match: None,
                    resolves: None,
                },
            )
            .staged(object.clone()),
        });
    }
    let enqueued = producer.enqueue_batch(&batch).unwrap();
    drop(producer);
    std::fs::remove_file(store.blobs().path(&object.hash).unwrap()).unwrap();

    let third = items / 3;
    let report = store.drain().unwrap();
    assert_eq!(
        (
            report.applied,
            report.parked,
            report.retried,
            report.skipped
        ),
        (third, third, third, 0)
    );

    let reader = io_pimdir::client::reader::PimdirReader::open(dir.path()).unwrap();
    for (at, row_) in enqueued.iter().enumerate() {
        let status = reader.action_status(&row_.handle).unwrap();
        match at % 3 {
            0 => assert!(
                matches!(status, PimdirActionStatus::Applied { .. }),
                "row {at}: {status:?}"
            ),
            1 => assert!(
                matches!(status, PimdirActionStatus::Parked { attempts: 1, .. }),
                "row {at}: {status:?}"
            ),
            _ => assert!(
                matches!(status, PimdirActionStatus::Pending { attempts: 1, .. }),
                "row {at}: {status:?}"
            ),
        }
    }
    for (at, seq) in seqs.iter().enumerate() {
        let item = store.get_item("INBOX", *seq).unwrap().unwrap();
        assert_eq!(item.flags.contains("\\Seen"), at % 3 == 0, "item {at}");
        assert_eq!(item.object, Some(PimdirHash(SEEDED.into())), "item {at}");
    }

    // the rows left pending keep their place: the next drain meets them
    let report = store.drain().unwrap();
    assert_eq!(
        (report.applied, report.parked, report.retried),
        (0, 0, third)
    );
}
