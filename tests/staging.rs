//! A producer's own body and creates (STORAGE §8, §15.1, §15.3): a body
//! it stages stays in the blob tree until the row that pins it, and a
//! create naming a link id the collection holds or has queued is refused
//! before anything is appended.

#![cfg(feature = "client")]

use std::{io::Write, path::Path};

use io_pimdir::{
    client::{
        PimdirDuplicate, PimdirError, PimdirSourceStore, PimdirStore,
        producer::{PimdirActionStatus, PimdirProducer},
    },
    codec::PimdirAction,
    placement::{PimdirFlags, PimdirLinkId},
};

const BODY: &[u8] = b"Message-ID: <new@x>\r\nSubject: hi\r\n\r\nnew body";

fn opened(dir: &Path) -> PimdirSourceStore {
    let store = PimdirStore::open(dir).unwrap().for_source("local");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    store
}

fn link(id: &str) -> PimdirLinkId {
    PimdirLinkId(id.into())
}

/// The window a reader's blob handle left open: a body written there and
/// swept by a collector before the enqueue was lost, the drain retrying an
/// add whose body was gone. A staged body holds the producer's lock until
/// its row pins it.
#[test]
fn a_body_staged_before_its_enqueue_survives_a_collector() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let object = producer.stage_body(BODY).unwrap();
    assert_eq!(object.hash, store.hash(BODY));
    assert_eq!(object.size, BODY.len());

    assert!(matches!(
        store.collect_garbage(),
        Err(PimdirError::Staging(_))
    ));

    producer
        .enqueue_add(
            "INBOX",
            &link("mid:new@x"),
            PimdirFlags::default(),
            Some(&object),
            None,
        )
        .unwrap();
    drop(producer);

    assert_eq!(store.collect_garbage().unwrap().blobs, 0);
    assert_eq!(store.drain().unwrap().applied, 1);
    assert_eq!(
        store.blobs().get(&object.hash).unwrap().as_deref(),
        Some(BODY)
    );
}

/// A streamed body is named by the bytes written, and its writer keeps the
/// staging lock past the producer that opened it.
#[test]
fn a_streamed_body_holds_the_lock_until_its_commit() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());

    let producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let mut writer = producer.blob_writer().unwrap();
    drop(producer);
    writer.write_all(&BODY[..10]).unwrap();
    writer.write_all(&BODY[10..]).unwrap();

    assert!(matches!(
        store.collect_garbage(),
        Err(PimdirError::Staging(_))
    ));

    let object = writer.commit().unwrap();
    assert_eq!(object.hash, store.hash(BODY));
    assert_eq!(object.size, BODY.len());
    assert_eq!(
        store.blobs().get(&object.hash).unwrap().as_deref(),
        Some(BODY)
    );
    assert!(store.collect_garbage().is_ok());
}

#[test]
fn a_create_naming_a_live_or_pending_link_id_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();

    let add = |producer: &mut PimdirProducer, id: &str| {
        producer.enqueue_add("INBOX", &link(id), PimdirFlags::default(), None, None)
    };

    add(&mut producer, "mid:a").unwrap();
    assert_eq!(store.drain().unwrap().applied, 1);
    let seq = store.seq_for_link("INBOX", "mid:a").unwrap().unwrap();
    let pending = add(&mut producer, "mid:b").unwrap();

    match add(&mut producer, "mid:a") {
        Err(err @ PimdirError::Duplicate { .. }) => {
            assert_eq!(err.code(), "item-duplicate");
            let PimdirError::Duplicate { existing, .. } = err else {
                unreachable!()
            };
            assert_eq!(existing, PimdirDuplicate::Live(seq));
        }
        other => panic!("expected a duplicate, got {other:?}"),
    }
    assert!(matches!(
        add(&mut producer, "mid:b"),
        Err(PimdirError::Duplicate {
            existing: PimdirDuplicate::Pending(id),
            ..
        }) if id == pending
    ));
    assert_eq!(producer.pending_actions("INBOX").unwrap().len(), 1);

    let created = add(&mut producer, "mid:c").unwrap();
    assert!(matches!(
        producer.action_status(created).unwrap(),
        PimdirActionStatus::Pending { .. }
    ));

    // a live item a queued remove withdraws is no longer in the way
    producer
        .enqueue("INBOX", &PimdirAction::Remove { seq }, None)
        .unwrap();
    add(&mut producer, "mid:a").unwrap();
    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (4, 0));
}

/// Every blob file under objects/, by path.
fn blob_files(dir: &Path) -> Vec<std::path::PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(&dir.join("objects"), &mut files);
    files.sort();
    files
}

/// A create refused for its link id writes no body (STORAGE §15.1,
/// §15.3): the check runs on the bytes in hand, so the tree is left as it
/// was, where staging first left an unpinned body for the collector.
#[test]
fn a_create_refused_for_its_link_id_stages_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    producer
        .enqueue_with_body("INBOX", BODY, |hash| PimdirAction::Add {
            link_id: None,
            flags: PimdirFlags::default(),
            object: Some(hash),
            supersedes: None,
        })
        .unwrap();
    assert_eq!(blob_files(dir.path()).len(), 1);
    drop(producer);
    assert_eq!(store.drain().unwrap().applied, 1);
    let before = blob_files(dir.path());

    // the same key again, derived from another body, then named
    let other = b"Message-ID: <new@x>\r\nSubject: again\r\n\r\nanother body";
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    for link_id in [None, Some(link("new@x"))] {
        let refused = producer
            .enqueue_with_body("INBOX", other, |hash| PimdirAction::Add {
                link_id: link_id.clone(),
                flags: PimdirFlags::default(),
                object: Some(hash),
                supersedes: None,
            })
            .unwrap_err();
        assert!(
            matches!(refused, PimdirError::Duplicate { .. }),
            "{refused:?}"
        );
    }
    assert_eq!(blob_files(dir.path()), before, "objects/ unchanged");
    assert!(producer.pending_actions("INBOX").unwrap().is_empty());
}

/// A producer holds the staging lock for its staging window alone (STORAGE
/// §8): an always-open frontend that stages nothing keeps no collector out,
/// and one that staged a body keeps it out until the row pinning it.
#[test]
fn an_idle_producer_keeps_no_collector_out() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    assert!(store.collect_garbage().is_ok(), "an idle producer");

    let object = producer.stage_body(BODY).unwrap();
    assert!(matches!(
        store.collect_garbage(),
        Err(PimdirError::Staging(_))
    ));
    producer
        .enqueue_add(
            "INBOX",
            &link("mid:new@x"),
            PimdirFlags::default(),
            Some(&object),
            None,
        )
        .unwrap();
    assert!(store.collect_garbage().is_ok(), "the body is pinned");

    producer
        .enqueue_with_body("INBOX", b"Subject: two\r\n\r\ntwo", |hash| {
            PimdirAction::Add {
                link_id: Some(link("mid:two@x")),
                flags: PimdirFlags::default(),
                object: Some(hash),
                supersedes: None,
            }
        })
        .unwrap();
    assert!(store.collect_garbage().is_ok(), "the one-step enqueue");
    assert_eq!(store.drain().unwrap().applied, 2);
}

/// An app's check proves the store writable by its producer, the staging
/// lock included, without writing anything into the store (STORAGE §8): a
/// collector holding the lock is reported busy at once, never waited out.
#[test]
fn a_probe_takes_the_staging_lock_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    drop(opened(dir.path()));
    let producer = PimdirProducer::open(dir.path(), "app").unwrap();

    let collector = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.path().join("objects.lock"))
        .unwrap();
    fs4::FileExt::try_lock(&collector).unwrap();
    let Err(err) = producer.probe() else {
        panic!("probed beside a collector");
    };
    assert_eq!(err.code(), "store-busy");
    drop(collector);

    producer.probe().unwrap();
    assert!(blob_files(dir.path()).is_empty(), "nothing in objects/");

    // NOTE: the lock went with the probe.
    let collector = std::fs::File::open(dir.path().join("objects.lock")).unwrap();
    assert!(fs4::FileExt::try_lock(&collector).is_ok(), "released");
    drop(collector);

    // A producer inside its staging window holds the lock already.
    producer.stage_body(BODY).unwrap();
    producer.probe().unwrap();
}
