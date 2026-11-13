//! The action queue and collection generations: a producer's enqueue
//! against a real temp store, the owner's drain per action kind, parking,
//! refcount pinning of queued bodies, generation bumps, and the refusal
//! of a store stamped with an unserviced schema version.

#![cfg(feature = "client")]

use std::{fs, path::Path, sync::mpsc, thread, time::Duration};

use io_pimdir::{
    change::{PimdirDropReason, PimdirWriteOp},
    collection::{PimdirCheckpoint, PimdirCollectionId},
    load::PimdirLoadSelection,
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
};
use io_pimdir::{
    client::blobs::PimdirBlobs,
    client::producer::PimdirProducer,
    client::reader::PimdirReader,
    client::{PimdirError, PimdirSourceStore, PimdirStore},
    codec::{PimdirAction, PimdirActionError},
    hash, sql,
};

/// Restricts a store built by hand to its user, as a store this crate
/// creates is (STORAGE §18), so what a test meets is its schema.
#[cfg(unix)]
fn restrict(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(dir.join("pimdir.db"), fs::Permissions::from_mode(0o600)).unwrap();
}

/// Restricts a store built by hand to its user: nothing to do where no
/// mode is checked.
#[cfg(not(unix))]
fn restrict(_dir: &Path) {}

fn inbox() -> PimdirCollectionId {
    PimdirCollectionId("INBOX".into())
}

/// A hydrated, linked placement with a matching base (so it projects clean).
fn placement(handle: &str, link: &str, hash: &str, flags: &[&str]) -> PimdirPlacement {
    let flags = PimdirFlags::from_iter(flags.iter().copied());
    PimdirPlacement {
        sort_key: Default::default(),
        collection: inbox(),
        handle: PimdirHandle(handle.into()),
        link_id: Some(PimdirLinkId(link.into())),
        object: Some(PimdirHash(hash.into())),
        level: PimdirLevel::Full,
        summary: None,
        flags: flags.clone(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags,
            revision: None,
            object: Some(PimdirHash(hash.into())),
        }),
        origin: None,
    }
}

/// The placement with the summary a listing names a message by: a
/// message shares its public id across collections only with a row whose
/// summary agrees with its own, one holding none agreeing with no
/// message (STORAGE §9, §9.1).
fn summarized(mut placement: PimdirPlacement) -> PimdirPlacement {
    let subject = placement.link_id.as_ref().unwrap().0.clone();
    placement.summary = Some(io_pimdir::summary::PimdirSummary::Mail(
        io_pimdir::summary::mail::PimdirMailSummary {
            subject,
            ..Default::default()
        },
    ));
    placement
}

fn store_object(hash: &str, body: &[u8]) -> PimdirWriteOp {
    PimdirWriteOp::StoreObject {
        object: PimdirObject {
            hash: PimdirHash(hash.into()),
            size: body.len(),
        },
        body: Some(body.to_vec()),
    }
}

fn blob_exists(dir: &Path, hash: &str) -> bool {
    dir.join("objects").join(&hash[0..2]).join(hash).exists()
}

/// Seeds a one-item INBOX and returns `(store, seq)` for the item.
fn seeded(dir: &Path) -> (PimdirSourceStore, i64) {
    let mut store = PimdirStore::open(dir).unwrap().for_source("local");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    store
        .write(vec![
            store_object(
                "cafebabecafebabecafebabecafebabecafebabecafebabecafe",
                b"abc",
            ),
            PimdirWriteOp::UpsertPlacement(placement(
                "1",
                "mid:a",
                "cafebabecafebabecafebabecafebabecafebabecafebabecafe",
                &["\\Seen"],
            )),
        ])
        .unwrap();
    let seq = store.list_items("INBOX", None, 10).unwrap()[0].seq;
    (store, seq)
}

/// Places `body` in the blob tree under the name `hash`, a body a producer
/// staged, and returns the object the enqueue indexes.
/// Writes `body` to the blob tree under its content hash, as a producer
/// stages it, outside any lock.
fn write_blob(dir: &Path, body: &[u8]) -> PimdirObject {
    let hash = hash::hash(body);
    let path = PimdirBlobs::open(dir).path(&hash).unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, body).unwrap();
    PimdirObject {
        hash,
        size: body.len(),
    }
}

#[test]
fn a_queued_add_round_trips_into_a_staged_item() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, _) = seeded(dir.path());

    // the producer writes the blob durably first, then enqueues in one
    // transaction, and the pending action shows in its own reads
    let object = write_blob(dir.path(), b"Subject: hi\r\n\r\nnew body");
    let mut producer = PimdirProducer::open(dir.path(), "smtp").unwrap();
    let add = PimdirAction::Add {
        link_id: Some(PimdirLinkId("mid:new".into())),
        flags: PimdirFlags::from_iter(["\\Draft"]),
        object: Some(object.hash.clone()),
        supersedes: None,
    };
    producer.enqueue("INBOX", &add, Some(&object)).unwrap();

    let pending = producer.pending_actions("INBOX").unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].action, add);
    assert_eq!(pending[0].producer, "smtp");
    assert_eq!(store.list_pending_actions().unwrap().len(), 1);

    // the owner drains: the item lands, staged as a pending push
    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (1, 0));
    assert!(store.pending_actions("INBOX").unwrap().is_empty());
    assert!(store.list_pending_actions().unwrap().is_empty());

    let items = store.list_items("INBOX", None, 10).unwrap();
    let item = items.iter().find(|i| i.link_id.0 == "mid:new").unwrap();
    assert_eq!(item.object, Some(object.hash.clone()));
    assert_eq!(item.level, PimdirLevel::Full);
    assert!(item.flags.contains("\\Draft"));
    // the owner derived the summary from the body the producer wrote
    let read = store.get_item("INBOX", item.seq).unwrap().unwrap();
    assert_eq!(read.summary.as_ref().unwrap().title(), "hi");

    // projected as a base-less pending push, the shape a staged
    // in-process Add projects
    let projected = store
        .load(&inbox(), &PimdirLoadSelection::All)
        .unwrap()
        .placements;
    let created = projected
        .iter()
        .find(|p| p.link_id.as_ref().is_some_and(|l| l.0 == "mid:new"))
        .unwrap();
    assert_ne!(created.status, PimdirStatus::Clean, "a pending push");
    assert!(created.base.is_none(), "no prior sync: an append push");
}

/// STORAGE §15.3: a payload is valid whole or malformed. A producer
/// refuses a malformed one at the enqueue, naming the member, and the
/// owner parks one a producer queued without checking, never applying it
/// in part nor ignoring a member.
#[test]
fn a_malformed_payload_is_refused_at_the_enqueue_and_parked_by_the_drain() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq) = seeded(dir.path());
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();

    let bodiless = PimdirAction::Add {
        link_id: Some(PimdirLinkId("mid:new".into())),
        flags: PimdirFlags::default(),
        object: None,
        supersedes: None,
    };
    let Err(PimdirError::Action(PimdirActionError::MissingField("object"))) =
        producer.enqueue("INBOX", &bodiless, None)
    else {
        panic!("an add names its body");
    };
    let unread = PimdirAction::SetFlags {
        seq,
        flags: PimdirFlags::Unknown,
        base: None,
    };
    let Err(PimdirError::Action(PimdirActionError::Invalid { field: "flags", .. })) =
        producer.enqueue("INBOX", &unread, None)
    else {
        panic!("a flag set never read states nothing to set");
    };
    let annotated = PimdirAction::Unknown {
        kind: "submit".into(),
        payload: r#"{"v":1,"source":"smtp","from":"a@b.c","rcpts":["d@e.f"],"bcc":["g@h.i"]}"#
            .into(),
        object_hash: None,
    };
    let Err(PimdirError::Action(PimdirActionError::UnknownField(member))) =
        producer.enqueue("INBOX", &annotated, None)
    else {
        panic!("a member the version does not define is malformed");
    };
    assert_eq!(member, "bcc");
    assert!(producer.pending_actions("INBOX").unwrap().is_empty());
    drop(producer);

    // NOTE: a producer that does not check queues them all the same.
    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    for (action, payload) in [
        (
            "set-flags",
            format!(r#"{{"v":1,"seq":{seq},"flags":["\\Flagged"],"note":"by hand"}}"#),
        ),
        ("remove", r#"{"v":1,"seq":0}"#.into()),
        (
            "submit",
            r#"{"v":1,"from":"a@b.c","rcpts":["d@e.f"]}"#.into(),
        ),
    ] {
        conn.execute(
            "INSERT INTO queue(created_at, producer, collection, action, payload) \
             VALUES('2026-10-31T00:00:00.000Z', 'foreign', 'INBOX', ?1, ?2)",
            [action, payload.as_str()],
        )
        .unwrap();
    }
    drop(conn);

    let report = store.drain().unwrap();
    assert_eq!(
        (report.applied, report.parked, report.skipped),
        (0, 3, 0),
        "{report:?}"
    );
    let parked = store.parked_actions().unwrap();
    for (row, member) in parked.iter().zip(["note", "seq", "source"]) {
        assert!(row.error.contains(member), "{member}: {}", row.error);
    }
    let item = store.get_item("INBOX", seq).unwrap().unwrap();
    assert!(!item.flags.contains("\\Flagged"), "nothing applied in part");
}

#[test]
fn a_duplicate_add_parks_instead_of_clobbering() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, _) = seeded(dir.path());

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let object = producer.stage_body(b"Subject: again\r\n\r\nx\r\n").unwrap();
    let add = PimdirAction::Add {
        link_id: Some(PimdirLinkId("mid:a".into())),
        flags: PimdirFlags::default(),
        object: Some(object.hash.clone()),
        supersedes: None,
    };
    producer.enqueue("INBOX", &add, Some(&object)).unwrap();

    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (0, 1));
    let parked = store.parked_actions().unwrap();
    assert_eq!(parked.len(), 1);
    assert!(parked[0].error.contains("already present"), "{parked:?}");
    assert_eq!(parked[0].attempts, 1);
    // the live item is untouched
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
        flags: PimdirFlags::from_iter(["\\Flagged"]),
        base: None,
    };

    producer.enqueue("INBOX", &set, None).unwrap();
    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (1, 0));

    // absolute replacement: the old flag is gone, not merged
    let item = store.get_item("INBOX", seq).unwrap().unwrap();
    assert_eq!(item.flags, PimdirFlags::from_iter(["\\Flagged"]));

    // reapplying the same absolute set is a no-op, never an error
    producer.enqueue("INBOX", &set, None).unwrap();
    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (1, 0));
    let item = store.get_item("INBOX", seq).unwrap().unwrap();
    assert_eq!(item.flags, PimdirFlags::from_iter(["\\Flagged"]));
}

#[test]
fn remove_hides_the_item_and_an_absent_remove_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq) = seeded(dir.path());
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();

    producer
        .enqueue(
            "INBOX",
            &PimdirAction::Remove {
                seq,
                if_match: None,
                resolves: None,
            },
            None,
        )
        .unwrap();
    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (1, 0));

    // hidden from reads, kept as a tombstone for the sync to push
    assert!(store.get_item("INBOX", seq).unwrap().is_none());
    let projected = store
        .load(&inbox(), &PimdirLoadSelection::All)
        .unwrap()
        .placements;
    assert_eq!(projected[0].status, PimdirStatus::Tombstone);

    // removing it again is success, not a park
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::Remove {
                seq,
                if_match: None,
                resolves: None,
            },
            None,
        )
        .unwrap();
    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (1, 0));
    assert!(store.parked_actions().unwrap().is_empty());
}

#[test]
fn copy_fills_the_target_and_move_also_empties_the_source() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");
    // NOTE: a move or a copy into a collection nothing declared parks
    // (STORAGE §15.3), and a copy shares the public id of its source of
    // the same kind alone (§9.1), so every collection is declared first.
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    store.ensure_collection("Backup", "message/rfc822").unwrap();
    store
        .ensure_collection("Archive", "message/rfc822")
        .unwrap();
    store
        .write(vec![
            store_object("cafebabecafebabecafebabecafebabecafebabecafebabecafe", b"a"),
            store_object("cafebabfcafebabfcafebabfcafebabfcafebabfcafebabfcafe", b"b"),
            PimdirWriteOp::UpsertPlacement(summarized(placement(
                "1",
                "mid:a",
                "cafebabecafebabecafebabecafebabecafebabecafebabecafe",
                &[],
            ))),
            PimdirWriteOp::UpsertPlacement(summarized(placement(
                "2",
                "mid:b",
                "cafebabfcafebabfcafebabfcafebabfcafebabfcafebabfcafe",
                &[],
            ))),
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
                to: PimdirCollectionId("Backup".into()),
            },
            None,
        )
        .unwrap();
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::Move {
                seq: seq_b,
                to: PimdirCollectionId("Archive".into()),
            },
            None,
        )
        .unwrap();

    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (2, 0));

    // copy: the source keeps the item, the target gains it under the
    // same seq
    assert_eq!(store.count_items("INBOX").unwrap(), 1, "mid:b moved away");
    let backup = store.list_items("Backup", None, 10).unwrap();
    assert_eq!(backup.len(), 1);
    assert_eq!(backup[0].link_id.0, "mid:a");
    assert_eq!(backup[0].seq, seq_a, "a message keeps one public id");

    // move: the source empties, the target fills
    let archive = store.list_items("Archive", None, 10).unwrap();
    assert_eq!(archive.len(), 1);
    assert_eq!(archive[0].link_id.0, "mid:b");
    assert_eq!(archive[0].seq, seq_b);
    assert!(store.get_item("INBOX", seq_b).unwrap().is_none());
}

#[test]
fn a_queued_bodiless_move_applies_into_a_target_its_source_syncs() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    store.ensure_collection("Trash", "message/rfc822").unwrap();
    store
        .ensure_collection("Archive", "message/rfc822")
        .unwrap();
    let listed = |handle: &str, link: &str| {
        let mut listed = placement(handle, link, "", &[]);
        listed.object = None;
        listed.level = PimdirLevel::Meta;
        listed.base = Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: None,
            object: None,
        });
        PimdirWriteOp::UpsertPlacement(listed)
    };
    store
        .write(vec![
            listed("1", "mid:a"),
            listed("2", "mid:b"),
            PimdirWriteOp::SetCheckpoint {
                collection: PimdirCollectionId("Trash".into()),
                checkpoint: PimdirCheckpoint(b"cp".to_vec()),
            },
        ])
        .unwrap();
    let seq_of = |link: &str| {
        store
            .list_items("INBOX", None, 10)
            .unwrap()
            .into_iter()
            .find(|i| i.link_id.0 == link)
            .unwrap()
            .seq
    };
    let (seq_a, seq_b) = (seq_of("mid:a"), seq_of("mid:b"));

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    for (seq, to) in [(seq_a, "Trash"), (seq_b, "Archive")] {
        let to = PimdirCollectionId(to.into());
        producer
            .enqueue("INBOX", &PimdirAction::Move { seq, to }, None)
            .unwrap();
    }

    let report = store.drain().unwrap();
    assert_eq!(
        (report.applied, report.parked),
        (1, 1),
        "Trash, which the source syncs, takes it; Archive cannot"
    );
    let trash = store.list_items("Trash", None, 10).unwrap();
    assert_eq!(trash.len(), 1);
    assert_eq!(trash[0].link_id.0, "mid:a");
    assert!(store.list_items("Archive", None, 10).unwrap().is_empty());
}

#[test]
fn update_repoints_the_body() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq) = seeded(dir.path());

    let object = write_blob(dir.path(), b"edited body");
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::Update {
                seq,
                object: object.hash.clone(),
                if_match: None,
                resolves: None,
            },
            Some(&object),
        )
        .unwrap();

    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (1, 0));

    let item = store.get_item("INBOX", seq).unwrap().unwrap();
    assert_eq!(item.object, Some(object.hash.clone()));
    // the old body survives: the sync base still references it
    assert!(blob_exists(
        dir.path(),
        "cafebabecafebabecafebabecafebabecafebabecafebabecafe"
    ));
    assert!(blob_exists(dir.path(), &object.hash.0));
    // an immutable kind never owes a body (SYNC §9): the base follows the
    // edit and nothing is left to push
    let projected = store
        .load(&inbox(), &PimdirLoadSelection::All)
        .unwrap()
        .placements;
    assert_eq!(projected[0].status, PimdirStatus::Clean);
    assert_eq!(
        projected[0].base.as_ref().and_then(|b| b.object.clone()),
        Some(object.hash.clone()),
    );
}

#[test]
fn a_parked_action_does_not_block_later_actions() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq) = seeded(dir.path());
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();

    // an unappliable action, then a valid one behind it
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::Remove {
                seq: 9999,
                if_match: None,
                resolves: None,
            },
            None,
        )
        .unwrap();
    // NOTE: remove-of-absent succeeds, so the parking case is set-flags.
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::SetFlags {
                seq: 9999,
                flags: PimdirFlags::default(),
                base: None,
            },
            None,
        )
        .unwrap();
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::SetFlags {
                seq,
                flags: PimdirFlags::from_iter(["\\Answered"]),
                base: None,
            },
            None,
        )
        .unwrap();

    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (2, 1));

    // the later action applied despite the parked one before it
    let item = store.get_item("INBOX", seq).unwrap().unwrap();
    assert!(item.flags.contains("\\Answered"));

    // the parked row is recorded, queryable and skipped by the next pass
    let parked = store.parked_actions().unwrap();
    assert_eq!(parked.len(), 1);
    assert_eq!(parked[0].collection, "INBOX");
    assert_eq!(parked[0].action, "set-flags");
    assert!(parked[0].error.contains("unknown seq"), "{parked:?}");
    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (0, 0));
    assert_eq!(store.parked_actions().unwrap().len(), 1, "never deleted");
}

#[test]
fn gc_never_sweeps_a_queued_body() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seeded_seq) = seeded(dir.path());

    // a producer stages a body and enqueues the add referencing it
    let object = write_blob(dir.path(), b"queued body");
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::Add {
                link_id: Some(PimdirLinkId("mid:new".into())),
                flags: PimdirFlags::default(),
                object: Some(object.hash.clone()),
                supersedes: None,
            },
            Some(&object),
        )
        .unwrap();

    // the producer's staging window closed with the commit pinning the
    // body (tests/staging.rs proves the window itself), so a collector may
    // run beside the open producer
    assert!(store.collect_garbage().is_ok());
    drop(producer);

    // the owner retires and purges the seeded item, orphaning its body,
    // and collects: the queued body survives the sweep, pinned by its row
    store
        .write(vec![PimdirWriteOp::DropPlacement {
            collection: inbox(),
            handle: PimdirHandle("1".into()),
            reason: PimdirDropReason::Deleted,
        }])
        .unwrap();
    assert!(store.purge(inbox(), seeded_seq).unwrap());

    let collected = store.collect_garbage().unwrap();
    assert_eq!((collected.objects, collected.blobs), (1, 1));
    assert!(
        !blob_exists(
            dir.path(),
            "cafebabecafebabecafebabecafebabecafebabecafebabecafe"
        ),
        "the orphan is taken"
    );
    assert!(
        blob_exists(dir.path(), &object.hash.0),
        "the queued body is not"
    );

    // draining hands the pin over to the applied item
    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (1, 0));
    assert!(
        blob_exists(dir.path(), &object.hash.0),
        "now the item pins it"
    );

    // the hand-over was exact, so retiring and purging the item's only
    // placement orphans the body and the next collection takes it, with
    // no leaked queue pin
    let seq = store.seq_for_link("INBOX", "mid:new").unwrap().unwrap();
    store
        .write(vec![PimdirWriteOp::DropPlacement {
            collection: inbox(),
            handle: PimdirHandle("\u{1}mid:new".into()),
            reason: PimdirDropReason::Deleted,
        }])
        .unwrap();
    assert!(store.purge(inbox(), seq).unwrap());
    assert_eq!(store.collect_garbage().unwrap().blobs, 1);
    assert!(!blob_exists(dir.path(), &object.hash.0), "no refcount leak");
}

#[test]
fn an_unknown_kind_is_skipped_and_never_blocks_the_queue() {
    // the store defines no semantics for a `submit` intent but another
    // owner does, so skipping leaves it pending for that owner, where
    // parking would claim nobody can ever apply it
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq) = seeded(dir.path());
    let mut producer = PimdirProducer::open(dir.path(), "himalaya").unwrap();

    let object = write_blob(dir.path(), b"a message to send");
    let submit = PimdirAction::Unknown {
        kind: "submit".into(),
        payload: r#"{"v":1,"source":"smtp","from":"me@b.c","rcpts":["a@b.c"]}"#.into(),
        object_hash: Some(object.hash.clone()),
    };
    let id = producer
        .enqueue("INBOX", &submit, Some(&object))
        .unwrap()
        .handle
        .id;
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::SetFlags {
                seq,
                flags: PimdirFlags::from_iter(["\\Answered"]),
                base: None,
            },
            None,
        )
        .unwrap();

    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked, report.skipped), (1, 0, 1));

    // the action behind it applied all the same
    assert!(
        store
            .get_item("INBOX", seq)
            .unwrap()
            .unwrap()
            .flags
            .contains("\\Answered")
    );

    // the intent is still pending, never parked, readable whole by the
    // owner that can perform it, and its body still pinned
    assert!(store.parked_actions().unwrap().is_empty());
    let pending = store.pending_actions("INBOX").unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, id);
    assert_eq!(pending[0].action, submit);
    assert!(blob_exists(dir.path(), &object.hash.0));

    // a second pass skips it again rather than accumulating attempts
    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked, report.skipped), (0, 0, 1));
    assert_eq!(store.pending_actions("INBOX").unwrap()[0].attempts, 0);
}

#[test]
fn an_acknowledged_action_releases_its_queued_body() {
    // the other half of skip-not-park: the owner that performed the
    // intent out of band takes the row away, and its body's pin with it
    let dir = tempfile::tempdir().unwrap();
    let (mut store, _) = seeded(dir.path());
    let mut producer = PimdirProducer::open(dir.path(), "himalaya").unwrap();

    let object = write_blob(dir.path(), b"sent already");
    let id = producer
        .enqueue(
            "INBOX",
            &PimdirAction::Unknown {
                kind: "submit".into(),
                payload: r#"{"v":1,"source":"smtp","from":"me@b.c","rcpts":["a@b.c"]}"#.into(),
                object_hash: Some(object.hash.clone()),
            },
            Some(&object),
        )
        .unwrap()
        .handle
        .id;
    assert!(blob_exists(dir.path(), &object.hash.0));

    assert!(store.drop_action(id).unwrap());
    assert!(store.pending_actions("INBOX").unwrap().is_empty());
    assert!(blob_exists(dir.path(), &object.hash.0), "no write collects");

    drop(producer);
    assert_eq!(store.collect_garbage().unwrap().blobs, 1);
    assert!(
        !blob_exists(dir.path(), &object.hash.0),
        "the pin went with the row"
    );

    // acknowledging a row that is already gone reports nothing to drop
    assert!(!store.drop_action(id).unwrap());
}

#[test]
fn a_failed_action_retries_until_it_is_parked() {
    // the two failure shapes an owner reports itself: transient, still
    // pending with attempts advancing, and permanent, parked with the
    // reason
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq) = seeded(dir.path());
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let id = producer
        .enqueue(
            "INBOX",
            &PimdirAction::Remove {
                seq,
                if_match: None,
                resolves: None,
            },
            None,
        )
        .unwrap()
        .handle
        .id;

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

    // a parked row is still cancellable, and an unknown id is a no-op
    assert!(store.drop_action(id).unwrap());
    assert!(store.parked_actions().unwrap().is_empty());
    store.fail_action(id, Some("gone")).unwrap();
}

/// A parked row's error is recorded scrubbed of the credentials a
/// connector's error may carry (STORAGE §9.2, §18): a URL's userinfo, an
/// `Authorization` header, a token in a query string, the rest kept.
#[test]
fn a_parked_error_is_recorded_without_its_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let (store, seq) = seeded(dir.path());
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let id = producer
        .enqueue(
            "INBOX",
            &PimdirAction::Remove {
                seq,
                if_match: None,
                resolves: None,
            },
            None,
        )
        .unwrap()
        .handle
        .id;

    store
        .fail_action(
            id,
            Some(
                "STORE refused by imaps://ann:hunter2@mail.example.org/INBOX \
                 (GET https://dav.example.org/?access_token=s3cr3t, \
                 Authorization: Bearer eyJ0b2tlbg)",
            ),
        )
        .unwrap();

    let parked = store.parked_actions().unwrap();
    assert_eq!(
        parked[0].error,
        "STORE refused by imaps://***@mail.example.org/INBOX \
         (GET https://dav.example.org/?access_token=***, \
         Authorization: ***"
    );
}

#[test]
fn a_generation_bump_is_visible_to_a_reader() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();

    // the epoch starts at 1 and ordinary writes never bump it
    assert_eq!(store.generation("INBOX").unwrap(), Some(1));
    store
        .write(vec![
            store_object(
                "cafebabecafebabecafebabecafebabecafebabecafebabecafe",
                b"abc",
            ),
            PimdirWriteOp::UpsertPlacement(placement(
                "1",
                "mid:a",
                "cafebabecafebabecafebabecafebabecafebabecafebabecafe",
                &[],
            )),
        ])
        .unwrap();
    assert_eq!(store.generation("INBOX").unwrap(), Some(1));

    // a rekey bumps it in the same transaction: the item is re-bound
    // under its new handle and the epoch advances together
    let mut rekeyed = placement(
        "101",
        "mid:a",
        "cafebabecafebabecafebabecafebabecafebabecafebabecafe",
        &[],
    );
    rekeyed.status = PimdirStatus::Clean;
    store
        .write(vec![
            PimdirWriteOp::DropPlacement {
                collection: inbox(),
                handle: PimdirHandle("1".into()),
                // NOTE: a rekey replaces the row rather than deleting
                // the item; the same batch upserts it under its new
                // handle.
                reason: PimdirDropReason::Rekeyed,
            },
            PimdirWriteOp::UpsertPlacement(rekeyed),
        ])
        .unwrap();
    assert_eq!(store.generation("INBOX").unwrap(), Some(2));

    // and the item came with it: without this, the drop and the upsert
    // read as one source reporting an identity under a second handle, so
    // the binding would keep the handle the server had just voided
    let carried = store.load(&inbox(), &PimdirLoadSelection::All).unwrap();
    assert_eq!(carried.placements.len(), 1);
    assert_eq!(carried.placements[0].handle, PimdirHandle("101".into()));

    // a second reader handle over the same files observes the new epoch
    let reader = PimdirStore::open(dir.path()).unwrap();
    assert_eq!(reader.generation("INBOX").unwrap(), Some(2));
    let collections = reader.list_collections().unwrap();
    assert_eq!(collections[0].generation, 2);
    // an unknown collection has no epoch
    assert_eq!(reader.generation("nope").unwrap(), None);
}

#[test]
fn a_store_of_a_later_version_is_refused() {
    let dir = tempfile::tempdir().unwrap();

    // a fresh store lands at the current draft schema version, with
    // `user_version` and `store_meta.version` in agreement
    {
        let _store = PimdirStore::open(dir.path()).unwrap();
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

    // stamp the store as a later release would, its marker and both
    // stamps moved together
    {
        let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
        conn.execute_batch(
            "UPDATE store_meta SET format_version = 'storage-v2', version = 2;
             PRAGMA user_version = 2;",
        )
        .unwrap();
    }

    // every opener refuses it rather than half-reading: the owner, which
    // never migrates, the producer, and the read-only reader, which
    // refuses any version but the current one
    for result in [
        PimdirStore::open(dir.path()).map(drop),
        PimdirProducer::open(dir.path(), "test").map(drop),
        PimdirReader::open(dir.path()).map(drop),
    ] {
        match result {
            Err(err @ PimdirError::Version { .. }) => {
                assert!(
                    matches!(&err, PimdirError::Version { found: Some(found) } if found == "storage-v2")
                );
                assert_eq!(err.code(), "store-version");
                assert!(err.to_string().contains(sql::FORMAT_VERSION), "{err}");
            }
            Err(other) => panic!("expected a version refusal, got {other:?}"),
            Ok(()) => panic!("expected a version refusal, got an open store"),
        }
    }

    // a producer also refuses a store the owner has not created yet: it
    // never creates the schema, and neither does a reader, a directory
    // with no database at all included
    let fresh = tempfile::tempdir().unwrap();
    assert!(matches!(
        PimdirProducer::open(fresh.path(), "test"),
        Err(PimdirError::Uncreated)
    ));
    assert!(matches!(
        PimdirReader::open(fresh.path()),
        Err(PimdirError::Uncreated)
    ));
    rusqlite::Connection::open(fresh.path().join("pimdir.db")).unwrap();
    restrict(fresh.path());
    match PimdirProducer::open(fresh.path(), "test") {
        Err(PimdirError::Uncreated) => {}
        Err(other) => panic!("expected an uncreated-store refusal, got {other:?}"),
        Ok(_) => panic!("expected an uncreated-store refusal, got a producer"),
    }
}

/// STORAGE §15.2: the drain is store-wide, so the handle draining is not
/// the source an action is for. An action against an existing item runs as
/// the source binding it, and an add as a source syncing the collection,
/// so a mail account also syncing contacts and calendar, whose sources take
/// turns, never binds `caldav` to a mailbox or strands a mail action for
/// `imap` to find.
#[test]
fn a_drain_applies_an_action_as_the_source_of_its_collection() {
    let dir = tempfile::tempdir().unwrap();
    let (owner, seq) = seeded(dir.path());

    let mut producer = PimdirProducer::open(dir.path(), "frontend").unwrap();
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::SetFlags {
                seq,
                flags: PimdirFlags::default(),
                base: None,
            },
            None,
        )
        .unwrap();
    producer
        .enqueue_with_body("INBOX", b"Subject: queued\r\n\r\nx\r\n", |hash| {
            PimdirAction::Add {
                link_id: Some(PimdirLinkId("mid:queued".into())),
                flags: PimdirFlags::default(),
                object: Some(hash),
                supersedes: None,
            }
        })
        .unwrap();

    // A source with no binding in this collection drains first.
    let mut stranger = PimdirStore::open(dir.path()).unwrap().for_source("other");
    let report = stranger.drain().unwrap();
    assert_eq!((report.applied, report.skipped, report.parked), (2, 0, 0));

    let item = owner.get_item("INBOX", seq).unwrap().unwrap();
    assert_eq!(item.flags, PimdirFlags::default(), "the flag change landed");
    assert_eq!(
        owner.collection_sources("INBOX").unwrap(),
        vec!["local".to_string()],
        "the create is the binding source's to push, the stranger binds nothing"
    );
    let created = owner
        .load(&inbox(), &PimdirLoadSelection::All)
        .unwrap()
        .placements
        .into_iter()
        .find(|p| p.status == PimdirStatus::Created)
        .expect("the add staged for the binding source");
    assert_eq!(created.handle.as_str(), "\u{1}mid:queued");
}

/// STORAGE §15.2: a failure of the store is permanent for the row and
/// parks it, and neither it nor anything else stops the rows behind it.
/// The refused rebind is the spec's own example: an add of an identity
/// this source still binds, under another handle.
#[test]
fn a_store_failure_parks_the_row_and_the_pass_continues() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq_a) = seeded(dir.path());
    store
        .write(vec![
            store_object("cafebabfcafebabfcafebabfcafebabfcafebabfcafebabfcafe", b"b"),
            PimdirWriteOp::UpsertPlacement(placement(
                "2",
                "mid:b",
                "cafebabfcafebabfcafebabfcafebabfcafebabfcafebabfcafe",
                &[],
            )),
        ])
        .unwrap();
    let seq_b = store.seq_for_link("INBOX", "mid:b").unwrap().unwrap();
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();

    // tombstone mid:a, which keeps its binding on handle 1 for the push
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::Remove {
                seq: seq_a,
                if_match: None,
                resolves: None,
            },
            None,
        )
        .unwrap();
    assert_eq!(store.drain().unwrap().applied, 1);

    // an add of mid:a under another handle resolves the binding to a
    // second handle, which the store refuses (§10); a valid row follows
    producer
        .enqueue_with_body("INBOX", b"Subject: queued\r\n\r\nx\r\n", |hash| {
            PimdirAction::Add {
                link_id: Some(PimdirLinkId("mid:a".into())),
                flags: PimdirFlags::default(),
                object: Some(hash),
                supersedes: None,
            }
        })
        .unwrap();
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::SetFlags {
                seq: seq_b,
                flags: PimdirFlags::from_iter(["\\Answered"]),
                base: None,
            },
            None,
        )
        .unwrap();

    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked, report.skipped), (1, 1, 0));

    let parked = store.parked_actions().unwrap();
    assert_eq!(parked.len(), 1);
    assert_eq!(parked[0].action, "add");
    assert_eq!(parked[0].attempts, 1);
    assert!(parked[0].error.contains("holds handle 1"), "{parked:?}");
    let bindings = store.item_bindings("INBOX", "mid:a").unwrap();
    assert_eq!(bindings.len(), 1, "the refused write recorded nothing");
    assert!(
        store
            .get_item("INBOX", seq_b)
            .unwrap()
            .unwrap()
            .flags
            .contains("\\Answered"),
        "the row behind it applied"
    );
}

/// STORAGE §15.3: a `remove` needs no placement of the draining source.
/// It runs as the source binding the item, which tombstones it and pushes
/// the delete on its next run; already absent is success.
#[test]
fn a_remove_drained_by_a_stranger_tombstones_the_item_for_its_source() {
    let dir = tempfile::tempdir().unwrap();
    let (owner, seq) = seeded(dir.path());
    let mut producer = PimdirProducer::open(dir.path(), "frontend").unwrap();
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::Remove {
                seq,
                if_match: None,
                resolves: None,
            },
            None,
        )
        .unwrap();

    let mut stranger = PimdirStore::open(dir.path()).unwrap().for_source("other");
    let report = stranger.drain().unwrap();
    assert_eq!((report.applied, report.parked, report.skipped), (1, 0, 0));
    assert!(
        owner.get_item("INBOX", seq).unwrap().is_none(),
        "tombstoned"
    );
    assert!(owner.pending_actions("INBOX").unwrap().is_empty());

    let projected = owner
        .load(&inbox(), &PimdirLoadSelection::All)
        .unwrap()
        .placements;
    assert_eq!(projected[0].status, PimdirStatus::Tombstone);

    producer
        .enqueue(
            "INBOX",
            &PimdirAction::Remove {
                seq,
                if_match: None,
                resolves: None,
            },
            None,
        )
        .unwrap();
    let report = stranger.drain().unwrap();
    assert_eq!(
        (report.applied, report.parked, report.skipped),
        (1, 0, 0),
        "already absent is success"
    );
}

/// STORAGE §15.2: the pending list is read outside any transaction, so a
/// row it names may be gone by the time its turn comes; a claim that
/// deletes nothing is not this transaction's to apply, and not applied.
#[test]
fn a_claim_that_deletes_nothing_is_not_counted_as_applied() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq) = seeded(dir.path());
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let id = producer
        .enqueue(
            "INBOX",
            &PimdirAction::SetFlags {
                seq,
                flags: PimdirFlags::from_iter(["\\Answered"]),
                base: None,
            },
            None,
        )
        .unwrap()
        .handle
        .id;

    // another connection holds the write lock while the drain reads its
    // pending list, then takes the row away before the drain can claim it
    let db = dir.path().join("pimdir.db");
    let (held, holding) = mpsc::channel();
    let thief = thread::spawn(move || {
        let conn = rusqlite::Connection::open(db).unwrap();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        held.send(()).unwrap();
        thread::sleep(Duration::from_millis(300));
        conn.execute("DELETE FROM queue WHERE id = ?1", [id])
            .unwrap();
        conn.execute_batch("COMMIT").unwrap();
    });
    holding.recv().unwrap();

    let report = store.drain().unwrap();
    thief.join().unwrap();
    assert_eq!((report.applied, report.parked, report.skipped), (0, 0, 1));
    assert!(
        !store
            .get_item("INBOX", seq)
            .unwrap()
            .unwrap()
            .flags
            .contains("\\Answered"),
        "nothing applied"
    );
    assert!(store.pending_actions("INBOX").unwrap().is_empty());
}

/// STORAGE §15.4: one row a reader cannot decode is the drain's to park,
/// never a reason to fail every overlaid read.
#[test]
fn a_malformed_pending_row_is_skipped_by_the_overlay_and_parked_by_the_drain() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq) = seeded(dir.path());

    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute(
        "INSERT INTO queue(created_at, producer, collection, action, payload) \
         VALUES('2026-09-04T00:00:00.000Z', 'foreign', 'INBOX', 'set-flags', 'not json')",
        [],
    )
    .unwrap();
    drop(conn);
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::SetFlags {
                seq,
                flags: PimdirFlags::from_iter(["\\Flagged"]),
                base: None,
            },
            None,
        )
        .unwrap();

    // the overlay folds the row it can read and skips the other
    let reader = PimdirReader::open(dir.path()).unwrap().with_pending();
    let items = reader.list_items("INBOX", None, 10).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].flags, PimdirFlags::from_iter(["\\Flagged"]));
    assert_eq!(reader.count_items("INBOX").unwrap(), 1);
    // the strict read still says the row is there and unreadable
    assert!(matches!(
        reader.pending_actions("INBOX"),
        Err(PimdirError::Action(_))
    ));

    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked, report.skipped), (1, 1, 0));
    let parked = store.parked_actions().unwrap();
    assert_eq!(parked.len(), 1);
    assert_eq!(parked[0].producer, "foreign");
}

/// STORAGE §15.1: the enqueue pins the hash the action names, so a body
/// the store does not index yet has to come with its row.
#[test]
fn an_action_naming_an_unindexed_body_is_refused_at_the_enqueue() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, _) = seeded(dir.path());
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    // NOTE: the bytes sit in the blob tree, unindexed.
    let object = write_blob(dir.path(), b"new body");
    let add = PimdirAction::Add {
        link_id: Some(PimdirLinkId("mid:new".into())),
        flags: PimdirFlags::default(),
        object: Some(object.hash.clone()),
        supersedes: None,
    };

    assert!(matches!(
        producer.enqueue("INBOX", &add, None),
        Err(PimdirError::Sql(_))
    ));
    assert!(store.pending_actions("INBOX").unwrap().is_empty());

    producer.enqueue("INBOX", &add, Some(&object)).unwrap();
    assert_eq!(store.drain().unwrap().applied, 1);
}

/// STORAGE §3: a database lacking `store_meta` does not say it is a
/// pimdir store, and every role refuses it rather than failing the read of
/// its stamp.
#[test]
fn a_store_lacking_store_meta_is_foreign() {
    let dir = tempfile::tempdir().unwrap();
    drop(PimdirStore::open(dir.path()).unwrap());
    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch("DROP TABLE store_meta").unwrap();
    drop(conn);

    assert!(matches!(
        PimdirStore::open(dir.path()),
        Err(PimdirError::Foreign)
    ));
    assert!(matches!(
        PimdirReader::open(dir.path()),
        Err(PimdirError::Foreign)
    ));
}

/// Unlinks the blob of `hash`, as a collector running between a body's
/// write and its enqueue would (pimdir STORAGE §8).
fn unlink_blob(dir: &Path, hash: &str) {
    std::fs::remove_file(dir.join("objects").join(&hash[0..2]).join(hash)).unwrap();
}

/// A queued `add` whose body is gone is a failure of the environment
/// (pimdir STORAGE §15.2): retried, then parked, never applied as an item
/// naming a body the store lacks.
#[test]
fn a_queued_add_missing_its_body_is_retried_then_parked() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, _) = seeded(dir.path());
    let object = write_blob(dir.path(), b"Subject: hi\r\n\r\nnew body");
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let add = PimdirAction::Add {
        link_id: Some(PimdirLinkId("mid:new".into())),
        flags: PimdirFlags::default(),
        object: Some(object.hash.clone()),
        supersedes: None,
    };
    let id = producer
        .enqueue("INBOX", &add, Some(&object))
        .unwrap()
        .handle
        .id;
    drop(producer);
    unlink_blob(dir.path(), &object.hash.0);

    for attempt in 1..PimdirSourceStore::MAX_ATTEMPTS {
        let report = store.drain().unwrap();
        assert_eq!((report.applied, report.retried), (0, 1), "drain {attempt}");
        assert_eq!(store.pending_actions("INBOX").unwrap()[0].attempts, attempt);
    }
    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (0, 1));

    let parked = store.parked_actions().unwrap();
    assert_eq!(parked.len(), 1);
    assert_eq!(parked[0].id, id);
    assert!(
        parked[0].error.contains(&object.hash.0),
        "{}",
        parked[0].error
    );
    let items = store.list_items("INBOX", None, 10).unwrap();
    assert!(
        items.iter().all(|item| item.link_id.0 != "mid:new"),
        "no item created"
    );
}

/// The same for an `update`: the item keeps the body it had.
#[test]
fn a_queued_update_missing_its_body_is_retried_then_parked() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq) = seeded(dir.path());
    let object = write_blob(dir.path(), b"edited body");
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let update = PimdirAction::Update {
        seq,
        object: object.hash.clone(),
        if_match: None,
        resolves: None,
    };
    producer.enqueue("INBOX", &update, Some(&object)).unwrap();
    drop(producer);
    unlink_blob(dir.path(), &object.hash.0);

    for _ in 1..PimdirSourceStore::MAX_ATTEMPTS {
        let report = store.drain().unwrap();
        assert_eq!((report.applied, report.retried), (0, 1));
    }
    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (0, 1));

    let item = store.get_item("INBOX", seq).unwrap().unwrap();
    assert_eq!(
        item.object,
        Some(PimdirHash(
            "cafebabecafebabecafebabecafebabecafebabecafebabecafe".into()
        ))
    );
}

/// A card body under `uid` with full name `name`.
fn card(uid: &str, name: &str) -> Vec<u8> {
    format!("BEGIN:VCARD\r\nVERSION:4.0\r\nUID:{uid}\r\nFN:{name}\r\nEND:VCARD\r\n").into_bytes()
}

/// A drained edit a collection's policy settles against another body
/// keeps the loser as a retained item (SYNC §9), and the drain says so:
/// nothing else reports a body kept aside while the queue was applied.
#[test]
fn a_body_a_policy_overrules_during_a_drain_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let contacts = PimdirCollectionId("contacts".into());
    let bound = |handle: &str, body: &[u8]| {
        let hash = hash::hash(body);
        vec![
            PimdirWriteOp::StoreObject {
                object: PimdirObject {
                    hash: hash.clone(),
                    size: body.len(),
                },
                body: Some(body.to_vec()),
            },
            PimdirWriteOp::UpsertPlacement(PimdirPlacement {
                sort_key: Default::default(),
                collection: contacts.clone(),
                handle: PimdirHandle(handle.into()),
                link_id: Some(PimdirLinkId("u1".into())),
                object: Some(hash.clone()),
                level: PimdirLevel::Full,
                summary: None,
                flags: PimdirFlags::default(),
                status: PimdirStatus::Clean,
                conflict_revision: None,
                conflict_object: None,
                base: Some(PimdirBase {
                    flags: PimdirFlags::default(),
                    revision: Some("1".into()),
                    object: Some(hash),
                }),
                origin: None,
            }),
        ]
    };

    let store = PimdirStore::open(dir.path()).unwrap();
    store.ensure_collection("contacts", "text/vcard").unwrap();
    store
        .set_collection_conflict(
            "contacts",
            io_pimdir::hub::PimdirHubConflict::PreferIncoming,
        )
        .unwrap();
    let mut a = store.for_source("a");
    a.write(bound("a1", &card("u1", "Ann"))).unwrap();
    drop(a);
    let mut b = PimdirStore::open(dir.path()).unwrap().for_source("b");
    b.write(bound("b1", &card("u1", "Ann B"))).unwrap();
    let seq = b.list_items("contacts", None, 10).unwrap()[0].seq;
    let retained = b.list_retained("contacts", None, 10).unwrap().len();
    drop(b);

    let mut producer = PimdirProducer::open(dir.path(), "app").unwrap();
    let row = producer
        .enqueue_with_body("contacts", &card("u1", "Ann A"), |hash| {
            PimdirAction::Update {
                seq,
                object: hash,
                if_match: None,
                resolves: None,
            }
        })
        .unwrap()
        .handle
        .id;
    drop(producer);

    let mut owner = PimdirStore::open(dir.path()).unwrap().for_source("a");
    let report = owner.drain().unwrap();
    assert_eq!(report.applied, 1, "{report:?}");
    let kept = owner.list_retained("contacts", None, 10).unwrap();
    assert_eq!(kept.len(), retained + 1, "the policy kept a body aside");
    assert_eq!(report.overruled.len(), 1, "{report:?}");
    assert_eq!(report.overruled[0].id, row);
    assert_eq!(report.overruled[0].collection, "contacts");
    assert!(
        kept.iter()
            .any(|item| item.link_id == report.overruled[0].link_id)
    );
}

/// A queued flag change stating the set its producer read is applied as
/// the change it is (STORAGE §15.3, vectors/sync/141): a flag a sync
/// landed after the producer read survives the row, and the one the
/// producer removed goes. Stating no base, the same flags replace the set.
///
/// Before draft-10's second batch the payload carried no base and the
/// owner wrote the set over whatever the item held, erasing the flag
/// landed since.
#[test]
fn a_queued_flag_change_stating_its_base_keeps_a_flag_landed_since() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq) = seeded(dir.path());
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();

    // the producer read `\Seen` and answers the message
    let change = PimdirAction::SetFlags {
        seq,
        flags: PimdirFlags::from_iter(["\\Answered"]),
        base: Some(PimdirFlags::from_iter(["\\Seen"])),
    };
    producer.enqueue("INBOX", &change, None).unwrap();

    // a sync lands `\Flagged`, set on another device, before the drain
    let hash = "cafebabecafebabecafebabecafebabecafebabecafebabecafe";
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(placement(
            "1",
            "mid:a",
            hash,
            &["\\Flagged", "\\Seen"],
        ))])
        .unwrap();

    // the overlay folds the change over the set the item shows
    let reader = PimdirReader::open(dir.path()).unwrap().with_pending();
    let shown = reader.get_item("INBOX", seq).unwrap().unwrap();
    assert_eq!(
        shown.flags,
        PimdirFlags::from_iter(["\\Answered", "\\Flagged"])
    );

    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (1, 0));
    let item = store.get_item("INBOX", seq).unwrap().unwrap();
    assert_eq!(
        item.flags,
        PimdirFlags::from_iter(["\\Answered", "\\Flagged"]),
        "the flag landed since survives, the one removed goes"
    );

    // the same flags stating no base replace the set
    let replace = PimdirAction::SetFlags {
        seq,
        flags: PimdirFlags::from_iter(["\\Answered"]),
        base: None,
    };
    producer.enqueue("INBOX", &replace, None).unwrap();
    store.drain().unwrap();
    let item = store.get_item("INBOX", seq).unwrap().unwrap();
    assert_eq!(item.flags, PimdirFlags::from_iter(["\\Answered"]));
}

/// A queued flag change stating its base parks over an item whose flag
/// set is unknown (STORAGE §15.2, §15.3, vectors/sync/142): there is no
/// set to change, and replacing it would write the unread flags as
/// removed. The overlay folds nothing for it.
#[test]
fn a_queued_flag_change_stating_its_base_parks_over_an_unknown_set() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    let hash = "cafebabecafebabecafebabecafebabecafebabecafebabecafe";
    let mut unread = placement("1", "mid:a", hash, &[]);
    unread.flags = PimdirFlags::Unknown;
    unread.base.as_mut().unwrap().flags = PimdirFlags::Unknown;
    store
        .write(vec![
            store_object(hash, b"abc"),
            PimdirWriteOp::UpsertPlacement(unread),
        ])
        .unwrap();
    let seq = store.list_items("INBOX", None, 10).unwrap()[0].seq;

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let change = PimdirAction::SetFlags {
        seq,
        flags: PimdirFlags::from_iter(["\\Seen"]),
        base: Some(PimdirFlags::default()),
    };
    producer.enqueue("INBOX", &change, None).unwrap();

    let reader = PimdirReader::open(dir.path()).unwrap().with_pending();
    let shown = reader.get_item("INBOX", seq).unwrap().unwrap();
    assert_eq!(shown.flags, PimdirFlags::Unknown, "folded into nothing");

    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (0, 1));
    let parked = store.parked_actions().unwrap();
    assert!(
        parked[0].error.contains("unknown flag set"),
        "{}",
        parked[0].error
    );
    let item = store.get_item("INBOX", seq).unwrap().unwrap();
    assert_eq!(item.flags, PimdirFlags::Unknown, "the row is as it was");

    // an unknown set is no base a producer may state
    let unknown = PimdirAction::SetFlags {
        seq,
        flags: PimdirFlags::from_iter(["\\Seen"]),
        base: Some(PimdirFlags::Unknown),
    };
    assert!(matches!(
        producer.enqueue("INBOX", &unknown, None),
        Err(PimdirError::Action(PimdirActionError::Invalid {
            field: "base",
            ..
        }))
    ));
}

/// A queued `remove` stating the body it deletes parks once another write
/// replaced that body, naming both (STORAGE §15.3, vectors/sync/145), and
/// the overlay leaves the item in. Before draft-10's second batch a
/// remove stated no body and deleted whatever the item held.
#[test]
fn a_queued_remove_stating_its_body_parks_over_another() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq) = seeded(dir.path());
    let held = "cafebabecafebabecafebabecafebabecafebabecafebabecafe";
    let other = hash::hash(b"a body the item never held");
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();

    let stale = PimdirAction::Remove {
        seq,
        if_match: Some(other.clone()),
        resolves: None,
    };
    producer.enqueue("INBOX", &stale, None).unwrap();
    let reader = PimdirReader::open(dir.path()).unwrap().with_pending();
    assert!(
        reader.get_item("INBOX", seq).unwrap().is_some(),
        "the overlay folds nothing for a row the owner parks"
    );

    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (0, 1));
    let parked = store.parked_actions().unwrap();
    assert!(
        parked[0].error.contains(&other.0) && parked[0].error.contains(held),
        "{}",
        parked[0].error
    );
    assert!(store.get_item("INBOX", seq).unwrap().is_some(), "kept");

    let read = PimdirAction::Remove {
        seq,
        if_match: Some(PimdirHash(held.into())),
        resolves: None,
    };
    producer.enqueue("INBOX", &read, None).unwrap();
    assert!(reader.get_item("INBOX", seq).unwrap().is_none(), "folded");
    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (1, 0));
    assert!(store.get_item("INBOX", seq).unwrap().is_none());
}

/// A queued `remove` stating a resolution and meeting no conflict parks,
/// the conflict it was decided over being settled already (STORAGE
/// §15.3).
#[test]
fn a_queued_remove_resolving_no_conflict_parks() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq) = seeded(dir.path());
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();

    let resolution = PimdirAction::Remove {
        seq,
        if_match: None,
        resolves: Some(vec![hash::hash(b"a diverging body")]),
    };
    producer.enqueue("INBOX", &resolution, None).unwrap();
    let reader = PimdirReader::open(dir.path()).unwrap().with_pending();
    assert!(reader.get_item("INBOX", seq).unwrap().is_some(), "kept in");

    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (0, 1));
    let parked = store.parked_actions().unwrap();
    assert!(
        parked[0].error.contains("meets no conflict"),
        "{}",
        parked[0].error
    );
    assert!(store.get_item("INBOX", seq).unwrap().is_some());
}

/// A payload this owner does not know, its action or its version, is left
/// pending and untouched, the rows behind it applied, never parked, and a
/// reader lists it (STORAGE §15.2, §15.4; the standard's "skip" scenario):
/// a known action at `v` 2 whose members `v` 1 would read, one `v` 1
/// would refuse, an action the standard does not define and an
/// application's own.
///
/// Before draft-10's second batch a reader left out a row at a later
/// version, and a known action stating no version was parked.
#[test]
fn an_unknown_payload_stays_pending_untouched_and_listed() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq) = seeded(dir.path());

    let unknown = [
        ("remove", format!(r#"{{"v":2,"seq":{seq}}}"#)),
        ("set-flags", r#"{"v":2,"seq":"5"}"#.to_string()),
        ("update", r#"{"v":3,"seq":1}"#.to_string()),
        ("snooze", r#"{"seq":1}"#.to_string()),
        (
            "snooze",
            r#"{"v":1,"seq":1,"until":"2026-12-01T09:00:00Z"}"#.to_string(),
        ),
        ("x-archive", r#"{"v":3}"#.to_string()),
    ];
    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    for (action, payload) in &unknown {
        conn.execute(
            sql::ENQUEUE_ACTION,
            rusqlite::named_params! {
                ":producer": "later",
                ":collection": "INBOX",
                ":action": action,
                ":payload": payload,
                ":target_collection": None::<String>,
                ":object_hash": None::<String>,
            },
        )
        .unwrap();
    }
    let rows = |conn: &rusqlite::Connection| -> Vec<(i64, String, String, i64, Option<String>)> {
        conn.prepare("SELECT id, action, payload, attempts, error FROM queue ORDER BY id")
            .unwrap()
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let before = rows(&conn);

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let flag = PimdirAction::SetFlags {
        seq,
        flags: PimdirFlags::from_iter(["\\Answered"]),
        base: None,
    };
    producer.enqueue("INBOX", &flag, None).unwrap();

    let report = store.drain().unwrap();
    assert_eq!(
        (report.applied, report.parked, report.skipped),
        (1, 0, unknown.len())
    );
    assert!(store.parked_actions().unwrap().is_empty(), "never parked");
    assert_eq!(rows(&conn), before, "payload, attempts and error untouched");
    let item = store.get_item("INBOX", seq).unwrap().unwrap();
    assert!(item.flags.contains("\\Answered"), "the row behind applied");
    assert!(
        store.get_item("INBOX", seq).unwrap().is_some(),
        "the remove at v 2 was not read as v 1"
    );

    // a reader lists them, each with its action and its payload, and
    // folds them into nothing
    let reader = PimdirReader::open(dir.path()).unwrap().with_pending();
    let pending = reader.pending_actions("INBOX").unwrap();
    assert_eq!(pending.len(), unknown.len());
    for (listed, (action, payload)) in pending.iter().zip(&unknown) {
        assert_eq!(listed.action.kind(), *action);
        assert_eq!(
            &io_pimdir::codec::action_to_payload(&listed.action),
            payload
        );
        assert!(!listed.created_at.is_empty());
    }
    assert!(reader.get_item("INBOX", seq).unwrap().is_some());

    // the next drain lists them again, and nothing else
    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked, report.skipped), (0, 0, 6));
}

/// A payload of a known action stating no `v`, or one that is no integer,
/// is malformed (STORAGE §15.3): it states no version an owner could come
/// to know, so the drain parks it rather than leave it pending for ever,
/// and the row behind it applies.
#[test]
fn a_known_action_stating_no_usable_version_parks() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq) = seeded(dir.path());

    let malformed = [
        ("remove", format!(r#"{{"seq":{seq}}}"#)),
        ("remove", format!(r#"{{"v":"1","seq":{seq}}}"#)),
        ("remove", format!(r#"{{"v":1.5,"seq":{seq}}}"#)),
        ("remove", format!(r#"{{"v":null,"seq":{seq}}}"#)),
        ("remove", format!(r#"{{"v":true,"seq":{seq}}}"#)),
    ];
    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    for (action, payload) in &malformed {
        conn.execute(
            sql::ENQUEUE_ACTION,
            rusqlite::named_params! {
                ":producer": "careless",
                ":collection": "INBOX",
                ":action": action,
                ":payload": payload,
                ":target_collection": None::<String>,
                ":object_hash": None::<String>,
            },
        )
        .unwrap();
    }
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let flag = PimdirAction::SetFlags {
        seq,
        flags: PimdirFlags::from_iter(["\\Answered"]),
        base: None,
    };
    producer.enqueue("INBOX", &flag, None).unwrap();

    let report = store.drain().unwrap();
    assert_eq!(
        (report.applied, report.parked, report.skipped),
        (1, malformed.len(), 0)
    );
    let parked = store.parked_actions().unwrap();
    assert_eq!(parked.len(), malformed.len());
    for row in &parked {
        assert!(row.error.contains("version"), "{}", row.error);
        assert_eq!(row.attempts, 1);
    }
    let item = store
        .get_item("INBOX", seq)
        .unwrap()
        .expect("never removed");
    assert!(item.flags.contains("\\Answered"), "the row behind applied");
}

/// The enqueue answers a handle, the row's id under the identity of the
/// store that drew it (STORAGE §4.5, §15.4): it prints and parses back,
/// follows the row, and reads `Reset` once the store's incarnation moved,
/// where the bare id still reads whatever row holds it. A cancel given a
/// handle of another incarnation is refused and removes nothing.
#[test]
fn a_handle_follows_its_row_until_the_stores_identity_moves() {
    use io_pimdir::{client::producer::PimdirActionStatus, codec::PimdirActionHandle};

    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq) = seeded(dir.path());
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let flag = PimdirAction::SetFlags {
        seq,
        flags: PimdirFlags::from_iter(["\\Answered"]),
        base: None,
    };
    let handle = producer.enqueue("INBOX", &flag, None).unwrap().handle;

    let identity = store.change_cursor().unwrap();
    let stated = handle.store.clone().expect("the store's identity");
    assert_eq!(stated.store_id, identity.store_id);
    assert_eq!(stated.incarnation, identity.incarnation);
    let printed = handle.to_string();
    assert_eq!(
        printed,
        format!(
            "{}.{}.{}",
            identity.store_id, identity.incarnation, handle.id
        )
    );
    assert_eq!(printed.parse::<PimdirActionHandle>(), Ok(handle.clone()));
    assert_eq!(
        handle.id.to_string().parse::<PimdirActionHandle>(),
        Ok(PimdirActionHandle::from(handle.id))
    );
    assert!("1.2".parse::<PimdirActionHandle>().is_err());
    assert!("store.x.2".parse::<PimdirActionHandle>().is_err());

    let pending = |status: PimdirActionStatus| matches!(status, PimdirActionStatus::Pending { .. });
    assert!(pending(producer.action_status(&handle).unwrap()));
    assert!(pending(producer.action_status(handle.id).unwrap()));

    // a restore raises the incarnation: the handle names no row of this
    // store any more, whatever row holds its id
    store.raise_incarnation(None).unwrap();
    assert_eq!(
        producer.action_status(&handle).unwrap(),
        PimdirActionStatus::Reset
    );
    let reader = PimdirReader::open(dir.path()).unwrap();
    assert_eq!(
        reader.action_status(&handle).unwrap(),
        PimdirActionStatus::Reset
    );
    assert!(
        pending(producer.action_status(handle.id).unwrap()),
        "a bare id is followed unchecked"
    );

    let refused = store.drop_action(&handle).unwrap_err();
    assert_eq!(refused.code(), "store-reset");
    assert_eq!(store.list_pending_actions().unwrap().len(), 1, "kept");
    assert!(store.drop_action(handle.id).unwrap(), "a bare id cancels");

    // a handle answered under the new incarnation follows its row
    let handle = producer.enqueue("INBOX", &flag, None).unwrap().handle;
    assert!(pending(producer.action_status(&handle).unwrap()));
    assert!(store.drop_action(&handle).unwrap());
}
