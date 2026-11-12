//! The blob store of storage-draft-08 (pimdir STORAGE §5, §7): a name is
//! checked before it becomes a path, a write never replaces a file, no
//! link is followed under objects/, the collector unlinks only a body in
//! its own shard, a body failing its name is moved into quarantine/ by the
//! owner alone, and a queued body is read from its row.

#![cfg(feature = "client")]

use std::{
    fs,
    io::{ErrorKind, Write},
    path::Path,
};

use io_pimdir::{
    client::{
        PimdirError, PimdirSourceStore, PimdirStore, blobs::PimdirBlobStrayKind,
        producer::PimdirProducer, reader::PimdirReader,
    },
    codec::PimdirAction,
    object::{PimdirHash, PimdirObject},
    placement::PimdirFlags,
};

const CARD: &[u8] = b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:blob\r\nFN:Blob\r\nEND:VCARD\r\n";

/// A card longer than a copy's buffer, so a stream would hand on its
/// first bytes before the read reaching its end checks it.
#[cfg(feature = "cli")]
fn long_card() -> Vec<u8> {
    let note = "x".repeat(64 * 1024);
    format!("BEGIN:VCARD\r\nVERSION:4.0\r\nUID:long\r\nFN:Long\r\nNOTE:{note}\r\nEND:VCARD\r\n")
        .into_bytes()
}
const MAIL: &[u8] = b"Message-ID: <blob@x>\r\nSubject: blob\r\n\r\nbody";

/// An owner over a fresh store, its INBOX declared.
fn opened(dir: &Path) -> PimdirSourceStore {
    let store = PimdirStore::open(dir).unwrap().for_source("local");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    store
}

/// Flips the first byte of the file at `path`, as a disk fault would.
fn flip(path: &Path) {
    let mut bytes = fs::read(path).unwrap();
    bytes[0] ^= 1;
    fs::write(path, &bytes).unwrap();
}

/// The files quarantine/ holds.
fn quarantined(dir: &Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = fs::read_dir(dir.join("quarantine")) else {
        return Vec::new();
    };
    entries.map(|entry| entry.unwrap().path()).collect()
}

/// Enqueues an `add` of `body` in INBOX through a producer, answering the
/// object it names.
fn enqueue_add(dir: &Path, body: &[u8]) -> PimdirObject {
    let mut producer = PimdirProducer::open(dir, "test").unwrap();
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
    object
}

/// A body lives at objects/, the shard its first two characters name,
/// then its name: one shard level (§5).
#[test]
fn a_body_lives_one_shard_level_down() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let object = store.blob_stage().stage_body(CARD).unwrap();
    let name = &object.hash.0;

    let path = dir.path().join("objects").join(&name[..2]).join(name);
    assert!(path.is_file(), "{}", path.display());
    assert_eq!(store.blobs().path(&object.hash).unwrap(), path);
}

/// A name read from anywhere is 52 characters of the alphabet before a
/// path is built from it (§5): a read, a stream, a whole read and a path
/// of a malformed one are refused, nothing opened.
#[test]
fn a_malformed_name_is_refused_before_a_path_is_built() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let blobs = store.blobs();

    // NOTE: under the two-level layout this name walked out of objects/
    // into the store directory, where a file waits.
    fs::write(dir.path().join("outside"), b"not a body").unwrap();
    fs::create_dir_all(dir.path().join("objects/ab/cd/abcd")).unwrap();
    let escaping = PimdirHash::from("abcd/../../../../outside");

    let sound = blobs.hash(CARD).0;
    for name in [
        escaping.0.clone(),
        "../x".to_string(),
        sound.to_uppercase(),
        sound[..51].to_string(),
        format!("{sound}a"),
        format!("{}1", &sound[..51]),
    ] {
        let hash = PimdirHash(name.clone());
        let kind = |err: std::io::Error| err.kind();
        assert_eq!(
            blobs.path(&hash).map_err(kind),
            Err(ErrorKind::InvalidInput)
        );
        assert_eq!(
            blobs.get(&hash).map_err(kind),
            Err(ErrorKind::InvalidInput),
            "{name}"
        );
        assert_eq!(
            blobs.reader(&hash).map(|r| r.is_some()).map_err(kind),
            Err(ErrorKind::InvalidInput),
            "{name}"
        );
        assert_eq!(
            blobs.verified(&hash).map(|f| f.is_some()).map_err(kind),
            Err(ErrorKind::InvalidInput),
            "{name}"
        );
    }
}

/// A write moves its temporary onto the name by a move that fails when the
/// name exists, never by a rename replacing it (§5): what sits there is
/// verified, and a link, which holds no body, is moved aside whole by the
/// owner's write, never written through nor replaced in place, before the
/// body lands, staged or streamed.
#[cfg(unix)]
#[test]
fn a_write_never_replaces_what_holds_the_name() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let hash = store.hash(CARD);
    let path = store.blobs().path(&hash).unwrap();
    let shard = path.parent().unwrap().to_path_buf();

    // NOTE: a dangling link, which a write checking for a file before a
    // replacing rename takes for a free name.
    fs::create_dir_all(&shard).unwrap();
    std::os::unix::fs::symlink(dir.path().join("nowhere"), &path).unwrap();

    store.blob_stage().stage_body(CARD).unwrap();
    assert!(
        !dir.path().join("nowhere").exists(),
        "never written through"
    );
    assert_eq!(store.blobs().get(&hash).unwrap().as_deref(), Some(CARD));
    let moved = quarantined(dir.path());
    assert_eq!(moved.len(), 1, "the link set aside: {moved:?}");
    assert!(fs::symlink_metadata(&moved[0]).unwrap().is_symlink());

    // the streamed writer the same
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(dir.path().join("nowhere"), &path).unwrap();
    let mut writer = store.blob_stage().writer().unwrap();
    std::io::Write::write_all(&mut writer, CARD).unwrap();
    assert_eq!(writer.commit().unwrap().hash, hash);
    assert_eq!(store.blobs().get(&hash).unwrap().as_deref(), Some(CARD));
    assert_eq!(
        quarantined(dir.path()).len(),
        2,
        "the second link set aside"
    );

    let leftovers: Vec<_> = fs::read_dir(&shard).unwrap().collect();
    assert_eq!(leftovers.len(), 1, "no temporary left in the shard");
    assert!(
        !fs::read_dir(dir.path().join("objects"))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with('.')),
        "no temporary left in objects/"
    );
}

/// A name held by a file failing it is verified by every write finding it
/// (§5): a producer moves nothing, so its staged, streamed or enqueued
/// write fails naming it and enqueues nothing, the file left where it is;
/// the owner's next write sets it aside and lands the body.
#[test]
fn a_producer_meeting_a_failing_held_name_fails_its_write() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let hash = store.blob_stage().stage_body(CARD).unwrap().hash;
    let path = store.blobs().path(&hash).unwrap();
    // NOTE: part of the body, as a crash in an exclusive create and copy
    // leaves it.
    fs::write(&path, &CARD[..CARD.len() / 2]).unwrap();

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let held = |err: &PimdirError| matches!(err, PimdirError::Io(err) if err.kind() == ErrorKind::InvalidData && err.to_string().contains(&hash.0));
    let staged = producer.stage_body(CARD).unwrap_err();
    assert!(held(&staged), "{staged}");
    let mut writer = producer.blob_writer().unwrap();
    std::io::Write::write_all(&mut writer, CARD).unwrap();
    let streamed = writer.commit().unwrap_err();
    assert_eq!(streamed.kind(), ErrorKind::InvalidData, "{streamed}");
    let enqueued = producer
        .enqueue_with_body("INBOX", CARD, |object| PimdirAction::Add {
            link_id: None,
            flags: PimdirFlags::default(),
            object: Some(object),
            supersedes: None,
        })
        .unwrap_err();
    assert!(held(&enqueued), "{enqueued}");
    assert!(
        store.list_pending_actions().unwrap().is_empty(),
        "nothing enqueued"
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        &CARD[..CARD.len() / 2],
        "left in place"
    );
    assert!(
        quarantined(dir.path()).is_empty(),
        "a producer moves nothing"
    );
    drop(producer);

    store.blob_stage().stage_body(CARD).unwrap();
    assert_eq!(store.blobs().get(&hash).unwrap().as_deref(), Some(CARD));
    assert_eq!(quarantined(dir.path()).len(), 1, "the owner set it aside");
}

/// A link where a body belongs holds no body (§5): never read through, by
/// a whole read, a stream or a verified open, and moved into quarantine/
/// by the owner, the file it names left as it was.
#[cfg(unix)]
#[test]
fn a_link_as_a_blob_is_not_followed() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let hash = store.hash(CARD);
    let path = store.blobs().path(&hash).unwrap();

    let elsewhere = dir.path().join("elsewhere");
    fs::write(&elsewhere, CARD).unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &path).unwrap();

    let blobs = store.blobs();
    assert_eq!(blobs.get(&hash).unwrap(), None);
    assert!(blobs.reader(&hash).unwrap().is_none());
    assert!(blobs.verified(&hash).unwrap().is_none());
    let reader = PimdirReader::open(dir.path()).unwrap();
    assert_eq!(reader.blobs().get(&hash).unwrap(), None);
    assert!(
        fs::symlink_metadata(&path).is_ok(),
        "a reader moves nothing"
    );

    let moved = store.quarantine(&hash).unwrap().expect("moved");
    assert!(moved.starts_with(dir.path().join("quarantine")));
    assert!(fs::symlink_metadata(&path).is_err(), "the name is free");
    assert_eq!(fs::read(&elsewhere).unwrap(), CARD, "its target untouched");
}

/// A link where a shard belongs holds no body either (§5): nothing is read
/// through it nor written through it, and the owner moves it aside.
#[cfg(unix)]
#[test]
fn a_link_as_a_shard_is_not_followed() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let hash = store.hash(CARD);
    let path = store.blobs().path(&hash).unwrap();
    let shard = path.parent().unwrap().to_path_buf();

    let elsewhere = dir.path().join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    fs::write(elsewhere.join(&hash.0), CARD).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &shard).unwrap();

    let blobs = store.blobs();
    assert_eq!(blobs.get(&hash).unwrap(), None);
    assert!(blobs.reader(&hash).unwrap().is_none());
    let neighbour = (0..)
        .map(|i| format!("neighbour {i}").into_bytes())
        .find(|body| store.hash(body).0[..2] == hash.0[..2])
        .unwrap();
    let other = store.hash(&neighbour);
    assert!(
        store.blob_stage().stage_body(&neighbour).is_err(),
        "no write through it"
    );
    assert!(!elsewhere.join(&other.0).exists());
    assert!(
        store.blob_stage().stage_body(CARD).is_err(),
        "no name looked up through it"
    );

    let moved = store.quarantine(&hash).unwrap().expect("moved");
    assert!(fs::symlink_metadata(&moved).unwrap().is_symlink());
    assert!(fs::symlink_metadata(&shard).is_err(), "the shard is free");
    store.blob_stage().stage_body(CARD).unwrap();
    assert_eq!(blobs.get(&hash).unwrap().as_deref(), Some(CARD));
}

/// The collector unlinks a regular file, by lstat, whose well-formed name
/// begins with its shard's two characters, the shard a well-formed
/// directory and no link, and a period-prefixed one, a temporary no
/// writer holds under its locks (§5); anything else is left and reported.
#[test]
fn the_collector_leaves_what_is_no_body_and_reports_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());
    let objects = dir.path().join("objects");
    let orphan = store.blob_stage().stage_body(b"orphan").unwrap().hash;
    let name = |body: &[u8]| store.hash(body).0;

    // a well-formed name in another shard, and at the two-level path
    let misplaced = objects.join(&orphan.0[..2]).join(name(b"misplaced"));
    fs::write(&misplaced, b"misplaced").unwrap();
    let nested = name(b"nested");
    let two_level = objects.join(&nested[..2]).join(&nested[2..4]);
    fs::create_dir_all(&two_level).unwrap();
    fs::write(two_level.join(&nested), b"nested").unwrap();
    // a directory under a body's name, a shard named off the alphabet
    let dir_body = name(b"a directory");
    fs::create_dir_all(objects.join(&dir_body[..2]).join(&dir_body)).unwrap();
    let odd = name(b"odd shard");
    fs::create_dir_all(objects.join("A1")).unwrap();
    fs::write(objects.join("A1").join(&odd), b"odd shard").unwrap();
    // a temporary a writer killed mid-stream left
    fs::write(objects.join(".tmp-9-9-9"), b"in flight").unwrap();

    let report = store.collect_garbage().unwrap();
    assert_eq!(
        (report.blobs, report.temporaries, report.bytes),
        (1, 1, 6 + 9),
        "the orphan and the temporary"
    );
    assert!(!store.blobs().path(&orphan).unwrap().exists());
    assert!(!objects.join(".tmp-9-9-9").exists());
    for kept in [
        &misplaced,
        &two_level.join(&nested),
        &objects.join(&dir_body[..2]).join(&dir_body),
        &objects.join("A1"),
    ] {
        assert!(kept.exists(), "{} is left", kept.display());
    }
    assert_eq!(report.strays.len(), 4, "{:?}", report.strays);
    assert!(
        report
            .strays
            .iter()
            .all(|stray| stray.kind == PimdirBlobStrayKind::Foreign)
    );
}

/// A writer killed between its link and its unlink leaves a second name of
/// a whole body, and one killed mid-stream a part of one: under its locks
/// the collector unlinks both, the body staying, and a later check finds
/// no stray (§5). A link or a directory under a period-prefixed name is
/// nobody's temporary and stays.
#[test]
fn the_collector_unlinks_the_temporaries_a_crash_left() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());
    let objects = dir.path().join("objects");

    let pinned = enqueue_add(dir.path(), MAIL).hash;
    let held = store.blobs().path(&pinned).unwrap();
    let shard = held.parent().unwrap();

    let linked = shard.join(format!(".{}.9-9-9.tmp", pinned.0));
    fs::hard_link(&held, &linked).unwrap();
    let streamed = objects.join(".tmp-9-9-9");
    fs::write(&streamed, b"part of a body").unwrap();
    let hidden_dir = shard.join(".kept");
    fs::create_dir(&hidden_dir).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&held, objects.join(".link")).unwrap();

    let before = store.blobs().scan().unwrap();
    assert!(
        before
            .strays
            .iter()
            .any(|stray| stray.kind == PimdirBlobStrayKind::Temporary)
    );

    let report = store.collect_garbage().unwrap();
    assert_eq!((report.blobs, report.temporaries), (0, 2), "{report:?}");
    #[cfg(unix)]
    assert_eq!(report.bytes, 14, "the second name of a body frees nothing");
    assert!(!linked.exists() && !streamed.exists());
    assert_eq!(store.blobs().get(&pinned).unwrap().as_deref(), Some(MAIL));
    assert!(hidden_dir.is_dir(), "a directory is no temporary");
    #[cfg(unix)]
    assert!(fs::symlink_metadata(objects.join(".link")).is_ok());

    let after = store.blobs().scan().unwrap();
    assert!(
        after
            .strays
            .iter()
            .all(|stray| stray.kind != PimdirBlobStrayKind::Temporary),
        "{:?}",
        after.strays
    );
    assert_eq!(store.collect_garbage().unwrap().temporaries, 0);
}

/// A temporary a live writer holds is no crash leftover: a producer inside
/// its staging window, or a stream of the owner still open, makes the
/// collection fail and leaves the temporary where it is (§5, §8).
#[test]
fn a_live_writer_keeps_its_temporary() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());
    let temporaries = |dir: &Path| -> usize {
        fs::read_dir(dir.join("objects"))
            .unwrap()
            .filter(|entry| {
                let name = entry.as_ref().unwrap().file_name();
                name.to_string_lossy().starts_with('.')
            })
            .count()
    };

    let producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let mut writer = producer.blob_writer().unwrap();
    writer.write_all(b"streamed by a producer").unwrap();
    assert!(matches!(
        store.collect_garbage(),
        Err(PimdirError::Staging(_))
    ));
    assert_eq!(temporaries(dir.path()), 1);
    let streamed = writer.commit().unwrap();
    drop(producer);
    assert!(store.blobs().get(&streamed.hash).unwrap().is_some());

    let mut writer = store.blob_stage().writer().unwrap();
    writer.write_all(b"streamed by the owner").unwrap();
    assert!(matches!(
        store.collect_garbage(),
        Err(PimdirError::InFlight(_))
    ));
    assert_eq!(temporaries(dir.path()), 1);
    let streamed = writer.commit().unwrap();
    assert!(store.blobs().get(&streamed.hash).unwrap().is_some());

    let report = store.collect_garbage().unwrap();
    assert_eq!(report.temporaries, 0, "{report:?}");
    assert_eq!(report.blobs, 2, "two bodies no row names");
}

/// A link under objects/, a body's or a shard's, is never unlinked by the
/// collector, nor what it names: it is left and reported (§5).
#[cfg(unix)]
#[test]
fn the_collector_leaves_a_link() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());
    let objects = dir.path().join("objects");

    let target = dir.path().join("target");
    fs::write(&target, b"target").unwrap();
    let as_body = store.hash(b"target").0;
    fs::create_dir_all(objects.join(&as_body[..2])).unwrap();
    std::os::unix::fs::symlink(&target, objects.join(&as_body[..2]).join(&as_body)).unwrap();

    let shard_target = dir.path().join("shard-target");
    fs::create_dir(&shard_target).unwrap();
    let in_shard = store.hash(b"in a linked shard").0;
    fs::write(shard_target.join(&in_shard), b"in a linked shard").unwrap();
    let linked_shard = objects.join(&in_shard[..2]);
    if !linked_shard.exists() {
        std::os::unix::fs::symlink(&shard_target, &linked_shard).unwrap();
    }

    let report = store.collect_garbage().unwrap();
    assert_eq!(report.blobs, 0, "{report:?}");
    assert!(fs::symlink_metadata(objects.join(&as_body[..2]).join(&as_body)).is_ok());
    assert!(
        shard_target.join(&in_shard).exists(),
        "nothing through the link"
    );
    assert!(target.exists());
    assert!(
        report
            .strays
            .iter()
            .any(|stray| stray.kind == PimdirBlobStrayKind::Link)
    );
}

/// A body failing its name is missing to everyone, and only the owner
/// moves it, into quarantine/ under its name and a unique suffix, never
/// unlinked: a reader and a producer move nothing, and the collector never
/// walks quarantine/ (§7).
#[test]
fn the_owner_quarantines_a_corrupt_body_and_a_reader_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());
    let object = enqueue_add(dir.path(), MAIL);
    let path = store.blobs().path(&object.hash).unwrap();
    flip(&path);
    let corrupt = fs::read(&path).unwrap();

    let reader = PimdirReader::open(dir.path()).unwrap();
    assert_eq!(reader.blobs().get(&object.hash).unwrap(), None);
    let producer = PimdirProducer::open(dir.path(), "test").unwrap();
    assert_eq!(producer.blobs().get(&object.hash).unwrap(), None);
    drop(producer);
    assert!(path.exists(), "a reader and a producer move nothing");

    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.retried), (0, 1), "{report:?}");
    assert!(!path.exists(), "moved out of objects/");
    let moved = quarantined(dir.path());
    assert_eq!(moved.len(), 1);
    let file = moved[0].file_name().unwrap().to_string_lossy().to_string();
    assert!(file.starts_with(&format!("{}.", object.hash.0)), "{file}");
    assert_eq!(fs::read(&moved[0]).unwrap(), corrupt, "kept as it was");

    // the collector never walks quarantine/
    store.collect_garbage().unwrap();
    assert_eq!(quarantined(dir.path()), moved);

    // a second corruption is quarantined beside the first, never over it
    let producer = PimdirProducer::open(dir.path(), "test").unwrap();
    producer.stage_body(MAIL).unwrap();
    drop(producer);
    flip(&path);
    store.drain().unwrap();
    assert_eq!(quarantined(dir.path()).len(), 2);

    // the name free, the next write lands and the retried add applies
    let producer = PimdirProducer::open(dir.path(), "test").unwrap();
    producer.stage_body(MAIL).unwrap();
    drop(producer);
    assert_eq!(store.drain().unwrap().applied, 1);
    assert_eq!(store.count_items("INBOX").unwrap(), 1);
}

/// The user empties quarantine/ (§7) through the owner alone, under the
/// staging lock: a producer between a body and its row refuses it, and
/// once it is done every quarantined file goes, counted with its bytes.
#[test]
fn the_user_empties_quarantine_under_the_staging_lock() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());
    let object = enqueue_add(dir.path(), MAIL);
    flip(&store.blobs().path(&object.hash).unwrap());
    store.drain().unwrap();
    let moved = quarantined(dir.path());
    assert_eq!(moved.len(), 1);

    let producer = PimdirProducer::open(dir.path(), "test").unwrap();
    producer.stage_body(CARD).unwrap();
    let refused = store.empty_quarantine().err();
    assert!(
        matches!(refused, Some(PimdirError::Staging(_))),
        "{refused:?}"
    );
    assert_eq!(quarantined(dir.path()), moved, "nothing unlinked");
    drop(producer);

    let emptied = store.empty_quarantine().unwrap();
    assert_eq!((emptied.files, emptied.bytes), (1, MAIL.len() as u64));
    assert!(emptied.left.is_empty());
    assert!(quarantined(dir.path()).is_empty());
}

/// The drain reads a queued body from `queue.object_hash` and parks a row
/// whose payload's `object` names another (§15.3), naming both, rather
/// than reading the payload's.
#[test]
fn a_payload_restating_another_body_parks() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());
    let object = enqueue_add(dir.path(), MAIL);
    let other = store.blob_stage().stage_body(CARD).unwrap().hash;

    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    let payload = format!(r#"{{"v":1,"flags":[],"object":"{}"}}"#, other.0);
    conn.execute("UPDATE queue SET payload = ?1", [&payload])
        .unwrap();

    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (0, 1), "{report:?}");
    let error: String = conn
        .query_row("SELECT error FROM queue", [], |r| r.get(0))
        .unwrap();
    assert!(
        error.contains(&other.0) && error.contains(&object.hash.0),
        "{error}"
    );
    assert_eq!(store.count_items("INBOX").unwrap(), 0);
}

/// Before a push the owner verifies every body it uploads (STORAGE §7,
/// SYNC §5): one failing its name is left out and moved into quarantine/.
#[test]
fn the_verification_before_a_push_quarantines_a_corrupt_body() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let sound = store.blob_stage().stage_body(CARD).unwrap().hash;
    let corrupt = store.blob_stage().stage_body(MAIL).unwrap().hash;
    let path = store.blobs().path(&corrupt).unwrap();
    flip(&path);

    let verified = store.verify(&[sound.clone(), corrupt.clone()]).unwrap();
    assert_eq!(verified.into_iter().collect::<Vec<_>>(), vec![sound]);
    assert!(!path.exists());
    assert_eq!(quarantined(dir.path()).len(), 1);
}

/// A body served whole is verified before its first byte leaves (STORAGE
/// §7): `pimdir item export` of a corrupt body prints nothing.
#[cfg(feature = "cli")]
#[test]
fn an_export_serves_no_byte_of_a_corrupt_body() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap();
    store.ensure_collection("contacts", "text/vcard").unwrap();
    let card = long_card();
    let (_, seq) = store.put_item("contacts", &card).unwrap();
    let path = store.blobs().path(&store.hash(&card)).unwrap();
    drop(store);
    flip(&path);

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_pimdir"))
        .arg("--store")
        .arg(dir.path())
        .args(["item", "export", &seq.to_string()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    // NOTE: the CLI prints its error on stdout; no byte of the body.
    let printed = String::from_utf8_lossy(&output.stdout);
    assert!(!printed.contains("xxxx"), "{} bytes printed", printed.len());
}
