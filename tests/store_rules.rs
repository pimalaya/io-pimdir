//! Store rules of storage-draft-05 a write or a drain enforces (STORAGE
//! §7, §9, §14, §15): a collection holding queued actions is not deleted,
//! a hint beginning like a derived key is escaped, an unknown payload
//! version is skipped, an edit names the body it replaced, and a body is
//! checked against its name when read.

use std::path::Path;

use io_pimdir::{
    client::{PimdirError, PimdirSourceStore, PimdirStore, producer::PimdirProducer},
    codec::PimdirAction,
    placement::PimdirFlags,
};

fn opened(dir: &Path) -> PimdirSourceStore {
    let store = PimdirStore::open(dir).unwrap().for_source("local");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    store
}

/// The cascade used to drop a queued action with no receipt and no report
/// (STORAGE §14, `collections_keep_queued`): the delete is refused, typed,
/// and the row stays until the operator cancels it.
#[test]
fn a_collection_holding_a_queued_action_is_not_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let id = producer
        .enqueue(
            "INBOX",
            &PimdirAction::SetFlags {
                seq: 9999,
                flags: PimdirFlags::from_iter(["\\Seen"]),
            },
            None,
        )
        .unwrap();
    drop(producer);

    let refused = store.delete_collection("INBOX").unwrap_err();
    assert!(
        matches!(&refused, PimdirError::Queued { collection } if collection == "INBOX"),
        "{refused:?}"
    );
    assert_eq!(refused.code(), "collection-queued");
    assert_eq!(store.list_pending_actions().unwrap().len(), 1);
    assert!(
        store
            .list_collections()
            .unwrap()
            .iter()
            .any(|c| c.id == "INBOX")
    );

    // parked rows hold it too
    store.drain().unwrap();
    assert_eq!(store.parked_actions().unwrap().len(), 1);
    assert!(matches!(
        store.delete_collection("INBOX"),
        Err(PimdirError::Queued { .. })
    ));

    assert!(store.drop_action(id).unwrap());
    assert!(store.delete_collection("INBOX").unwrap());
}

/// A payload version the owner does not know, for a kind it knows, is a
/// later producer's (STORAGE §15.2): the row is skipped, left pending and
/// untouched, and the rows behind it apply.
#[test]
fn an_unknown_payload_version_is_skipped_not_parked() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());

    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute(
        io_pimdir::sql::ENQUEUE_ACTION,
        rusqlite::named_params! {
            ":producer": "later",
            ":collection": "INBOX",
            ":action": "set-flags",
            ":payload": r#"{"v":2,"seq":1,"flags":{"add":["\\Seen"]}}"#,
            ":object_hash": None::<String>,
        },
    )
    .unwrap();
    drop(conn);
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    producer
        .enqueue("INBOX", &PimdirAction::Remove { seq: 9999 }, None)
        .unwrap();
    drop(producer);

    let report = store.drain().unwrap();
    assert_eq!(
        (report.applied, report.parked, report.skipped),
        (1, 0, 1),
        "{report:?}"
    );
    assert!(store.parked_actions().unwrap().is_empty());
    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    let (attempts, error): (i64, Option<String>) = conn
        .query_row("SELECT attempts, error FROM queue", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!((attempts, error), (0, None), "untouched");
}

/// A body whose bytes no longer hash to its name is no body (STORAGE §7):
/// a read answers none, the drain applies nothing over it, and the owner
/// unlinks the file so the next write of that body writes it anew.
#[test]
fn a_body_failing_its_name_is_no_body_and_the_owner_unlinks_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());
    let body = b"Message-ID: <flip@x>\r\nSubject: flip\r\n\r\nbody".as_slice();

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let object = producer.stage_body(body).unwrap();
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::Add {
                link_id: None,
                flags: PimdirFlags::default(),
                object: Some(object.hash.clone()),
                supersedes: None,
            },
            Some(&object),
        )
        .unwrap();
    drop(producer);

    let path = store.blobs().path(&object.hash);
    let mut flipped = std::fs::read(&path).unwrap();
    flipped[0] ^= 1;
    std::fs::write(&path, &flipped).unwrap();

    assert_eq!(store.blobs().get(&object.hash).unwrap(), None);
    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.retried), (0, 1), "{report:?}");
    assert_eq!(store.count_items("INBOX").unwrap(), 0);
    assert!(!path.exists(), "the owner unlinked the corrupt file");

    // the producer writes it anew, and the retried add applies
    let producer = PimdirProducer::open(dir.path(), "test").unwrap();
    assert_eq!(producer.stage_body(body).unwrap(), object);
    drop(producer);
    assert_eq!(store.drain().unwrap().applied, 1);
    assert_eq!(store.count_items("INBOX").unwrap(), 1);
}

/// A body is written through a temporary named uniquely to its writer
/// (STORAGE §5): two processes storing one body never share one, so one
/// never renames the other's half-written bytes under a valid hash. A
/// temporary another writer holds is left alone.
#[test]
fn a_blob_write_never_takes_another_writers_temporary() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let body = b"one body, two writers".as_slice();
    let hash = store.hash(body);
    let path = store.blobs().path(&hash);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let theirs = path.parent().unwrap().join(format!(".{}.tmp", hash.0));
    std::fs::write(&theirs, b"one bo").unwrap();

    let producer = PimdirProducer::open(dir.path(), "test").unwrap();
    producer.stage_body(body).unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), body);
    assert_eq!(
        std::fs::read(&theirs).unwrap(),
        b"one bo",
        "the other writer's temporary is its own"
    );
}

/// An `update` names the body its producer read and replaced (STORAGE
/// §15.3): when another write replaced it since, the owner parks the row
/// naming both, rather than dropping that write without a merge.
#[test]
fn an_update_whose_read_body_was_replaced_parks() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());
    let read = b"Message-ID: <e@x>\r\nSubject: read\r\n\r\none".as_slice();
    let theirs = b"Message-ID: <e@x>\r\nSubject: theirs\r\n\r\ntwo".as_slice();
    let mine = b"Message-ID: <e@x>\r\nSubject: mine\r\n\r\nthree".as_slice();

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let object = producer.stage_body(read).unwrap();
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::Add {
                link_id: None,
                flags: PimdirFlags::default(),
                object: Some(object.hash.clone()),
                supersedes: None,
            },
            Some(&object),
        )
        .unwrap();
    drop(producer);
    store.drain().unwrap();
    let seq = store.list_items("INBOX", None, 1).unwrap()[0].seq;

    // two producers read `read`; the first replaces it, the second's
    // edit then names a body the item no longer holds
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    for body in [theirs, mine] {
        let object = producer.stage_body(body).unwrap();
        producer
            .enqueue(
                "INBOX",
                &PimdirAction::Update {
                    seq,
                    object: object.hash.clone(),
                    if_match: Some(store.hash(read)),
                },
                Some(&object),
            )
            .unwrap();
    }
    drop(producer);

    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (1, 1), "{report:?}");
    let item = store.get_item("INBOX", seq).unwrap().unwrap();
    assert_eq!(
        item.object,
        Some(store.hash(theirs)),
        "the first write kept"
    );
    let parked = store.parked_actions().unwrap();
    assert!(
        parked[0].error.contains(&store.hash(read).0)
            && parked[0].error.contains(&store.hash(theirs).0),
        "{}",
        parked[0].error
    );

    // a row stating none applies over whatever the item holds
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let object = producer.stage_body(mine).unwrap();
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::Update {
                seq,
                object: object.hash.clone(),
                if_match: None,
            },
            Some(&object),
        )
        .unwrap();
    drop(producer);
    assert_eq!(store.drain().unwrap().applied, 1);
    let item = store.get_item("INBOX", seq).unwrap().unwrap();
    assert_eq!(item.object, Some(store.hash(mine)));
}

/// A streamed read verifies at the end of the stream and fails it (STORAGE
/// §7): bytes no longer hashing to their name are never handed out as the
/// body, whole or streamed; a sound body streams as it is.
#[test]
fn a_streamed_body_failing_its_name_fails_at_its_end() {
    use std::io::Read;

    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let body = b"Message-ID: <stream@x>\r\nSubject: s\r\n\r\nbody".as_slice();
    let object = producer.stage_body(body).unwrap();
    drop(producer);

    let mut sound = Vec::new();
    let mut reader = store.blobs().reader(&object.hash).unwrap().unwrap();
    reader.read_to_end(&mut sound).unwrap();
    assert_eq!(sound, body);

    let path = store.blobs().path(&object.hash);
    let mut flipped = std::fs::read(&path).unwrap();
    flipped[0] ^= 1;
    std::fs::write(&path, &flipped).unwrap();

    let mut read = Vec::new();
    let mut reader = store.blobs().reader(&object.hash).unwrap().unwrap();
    let err = reader.read_to_end(&mut read).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData, "{err}");
}

/// A consumer reading exactly the length a body states, as a connector
/// streaming an upload of a known size does, never makes the read meeting
/// the end of the file: the read reaching that length verifies it (STORAGE
/// §7), and a body grown past it fails rather than streams unverified.
#[test]
fn a_streamed_body_read_to_its_length_is_verified() {
    use std::io::{ErrorKind, Read};

    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let body = b"Message-ID: <length@x>\r\nSubject: s\r\n\r\nbody".as_slice();
    let object = producer.stage_body(body).unwrap();
    drop(producer);

    let exactly = |hash| -> std::io::Result<Vec<u8>> {
        let mut reader = store.blobs().reader(hash).unwrap().unwrap();
        let mut read = vec![0; reader.len().unwrap() as usize];
        reader.read_exact(&mut read)?;
        Ok(read)
    };
    assert_eq!(exactly(&object.hash).unwrap(), body);

    let path = store.blobs().path(&object.hash);
    let mut flipped = std::fs::read(&path).unwrap();
    flipped[0] ^= 1;
    std::fs::write(&path, &flipped).unwrap();
    let err = exactly(&object.hash).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData, "{err}");

    // NOTE: the sound body back, then grown after the reader opened: what
    // follows the length it stated is not the body.
    std::fs::write(&path, body).unwrap();
    let mut reader = store.blobs().reader(&object.hash).unwrap().unwrap();
    let mut grown = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    std::io::Write::write_all(&mut grown, b"tail").unwrap();
    let mut read = Vec::new();
    let err = reader.read_to_end(&mut read).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData, "{err}");
}

/// The journal mode is chosen once, when the store is created (STORAGE
/// §4.1, §8): WAL for a local store, and a store switched to a rollback
/// journal for a network filesystem stays so, never flipped back to WAL by
/// an owner, a producer or a reader opening it.
#[test]
fn a_store_on_a_rollback_journal_stays_on_it() {
    let dir = tempfile::tempdir().unwrap();
    drop(opened(dir.path()));
    let mode = |dir: &Path| -> String {
        rusqlite::Connection::open(dir.join("pimdir.db"))
            .unwrap()
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap()
    };
    assert_eq!(mode(dir.path()), "wal", "a new store is created in WAL");

    rusqlite::Connection::open(dir.path().join("pimdir.db"))
        .unwrap()
        .pragma_update(None, "journal_mode", "DELETE")
        .unwrap();
    drop(PimdirProducer::open(dir.path(), "test").unwrap());
    assert_eq!(mode(dir.path()), "delete", "the producer");
    drop(io_pimdir::client::reader::PimdirReader::open(dir.path()).unwrap());
    assert_eq!(mode(dir.path()), "delete", "the reader");
    drop(PimdirStore::open_current(dir.path()).unwrap());
    assert_eq!(mode(dir.path()), "delete", "the current owner");
    drop(PimdirStore::open(dir.path()).unwrap());
    assert_eq!(mode(dir.path()), "delete", "the migrating owner");
}
