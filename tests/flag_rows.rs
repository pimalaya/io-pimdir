//! Flags as rows (STORAGE §4.3, §13): a set reads back whole in no order,
//! a set written again as it was stamps nothing, and a flag the store
//! cannot hold, empty or holding a space, a control or a non-ASCII
//! character, is refused as `flag-invalid` wherever it would enter: a
//! producer's enqueue, an owner's write, a drain of a row another producer
//! queued.

#![cfg(feature = "client")]

use std::path::Path;

use io_pimdir::{
    change::PimdirWriteOp,
    client::{PimdirError, PimdirSourceStore, PimdirStore, producer::PimdirProducer},
    codec::PimdirAction,
    collection::PimdirCollectionId,
    load::PimdirLoadScope,
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
};

fn inbox() -> PimdirCollectionId {
    PimdirCollectionId("INBOX".into())
}

fn placement(flags: PimdirFlags) -> PimdirPlacement {
    PimdirPlacement {
        sort_key: Default::default(),
        collection: inbox(),
        handle: PimdirHandle("1".into()),
        link_id: Some(PimdirLinkId("a@example.org".into())),
        object: Some(PimdirHash("cafebabe".into())),
        level: PimdirLevel::Full,
        summary: None,
        flags: flags.clone(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags,
            revision: None,
            object: Some(PimdirHash("cafebabe".into())),
        }),
        origin: None,
    }
}

fn write(store: &mut PimdirSourceStore, flags: PimdirFlags) -> Result<(), PimdirError> {
    store
        .write(vec![
            PimdirWriteOp::StoreObject {
                object: PimdirObject {
                    hash: PimdirHash("cafebabe".into()),
                    size: 3,
                },
                body: Some(b"abc".to_vec()),
            },
            PimdirWriteOp::UpsertPlacement(placement(flags)),
        ])
        .map(|_| ())
}

fn seeded(dir: &Path) -> PimdirSourceStore {
    let mut store = PimdirStore::open(dir).unwrap().for_source("imap");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    write(&mut store, PimdirFlags::from_iter(["\\Seen", "$label"])).unwrap();
    store
}

fn stamp(dir: &Path) -> i64 {
    rusqlite::Connection::open(dir.join("pimdir.db"))
        .unwrap()
        .query_row("SELECT changed FROM items", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn a_set_reads_back_whole_and_a_restatement_stamps_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seeded(dir.path());

    let loaded = store.load(&inbox(), &PimdirLoadScope::All).unwrap();
    let flags = PimdirFlags::from_iter(["$label", "\\Seen"]);
    assert_eq!(loaded.placements[0].flags, flags);
    assert_eq!(loaded.placements[0].base.as_ref().unwrap().flags, flags);

    let before = stamp(dir.path());
    write(&mut store, PimdirFlags::from_iter(["$label", "\\Seen"])).unwrap();
    assert_eq!(stamp(dir.path()), before, "the same set moved no row");

    write(&mut store, PimdirFlags::from_iter(["\\Seen"])).unwrap();
    assert!(stamp(dir.path()) > before, "a flag removed stamps the item");
}

#[test]
fn a_flag_the_store_cannot_hold_is_refused_at_the_write() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seeded(dir.path());
    let before = stamp(dir.path());

    for flag in ["two words", "", "tab\t", "caf\u{e9}"] {
        let refused = write(&mut store, PimdirFlags::from_iter(["\\Seen", flag])).unwrap_err();
        assert!(
            matches!(&refused, PimdirError::Flag(refused) if refused == flag),
            "{flag:?}: {refused:?}"
        );
        assert_eq!(refused.code(), "flag-invalid");
    }

    let loaded = store.load(&inbox(), &PimdirLoadScope::All).unwrap();
    assert_eq!(
        loaded.placements[0].flags,
        PimdirFlags::from_iter(["\\Seen", "$label"]),
        "nothing written, nothing trimmed"
    );
    assert_eq!(stamp(dir.path()), before);
}

#[test]
fn a_flag_the_store_cannot_hold_is_refused_at_the_enqueue_and_parked_by_the_drain() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seeded(dir.path());
    let seq = store.list_items("INBOX", None, 1).unwrap()[0].seq;

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let action = PimdirAction::SetFlags {
        seq,
        flags: PimdirFlags::from_iter(["two words"]),
    };
    let refused = producer.enqueue("INBOX", &action, None).unwrap_err();
    assert!(matches!(&refused, PimdirError::Flag(flag) if flag == "two words"));
    assert!(producer.pending_actions("INBOX").unwrap().is_empty());
    drop(producer);

    // NOTE: a producer of its own may still queue one; the owner parks it.
    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute(
        "INSERT INTO queue(created_at, producer, collection, action, payload) \
         VALUES('2026-10-31T00:00:00.000Z', 'foreign', 'INBOX', 'set-flags', ?1)",
        [format!(r#"{{"v":1,"seq":{seq},"flags":["two words"]}}"#)],
    )
    .unwrap();
    drop(conn);

    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (0, 1), "{report:?}");
    let parked = store.parked_actions().unwrap();
    assert!(parked[0].error.contains("two words"), "{parked:?}");
}
