//! An owner open that creates nothing (STORAGE §6, §8):
//! `PimdirStore::open_current` takes the owner lock on a store exactly as
//! this crate writes it, and refuses any other, leaving it as it was,
//! where `PimdirStore::open` would create an absent one. Neither
//! reconciles nor migrates: a store of another draft is refused by both.

#![cfg(feature = "client")]

use std::fs;

use io_pimdir::{
    client::{
        PimdirError, PimdirNotCurrent, PimdirStore, TIME_COLLECTION, producer::PimdirProducer,
        reader::PimdirReader,
    },
    codec::PimdirAction,
};

const ENTRY: &[u8] = b"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//test//EN\r\n\
BEGIN:VEVENT\r\nUID:entry-1@example.org\r\nDTSTAMP:20261028T090000Z\r\n\
DTSTART:20261028T090000Z\r\nDTEND:20261028T100000Z\r\nSUMMARY:Paint\r\n\
TRANSP:TRANSPARENT\r\nRELATED-TO:todo-1@example.org\r\n\
END:VEVENT\r\nEND:VCALENDAR\r\n";

#[test]
fn a_store_this_crate_created_opens_and_takes_writes() {
    let dir = tempfile::tempdir().unwrap();
    drop(PimdirStore::open(dir.path()).unwrap());

    let mut store = PimdirStore::open_current(dir.path()).unwrap();
    store.ensure_time_collection().unwrap();
    let (_, seq) = store.put_item(TIME_COLLECTION, ENTRY).unwrap();
    drop(store);

    let reader = PimdirReader::open(dir.path()).unwrap();
    assert!(reader.get_item(TIME_COLLECTION, seq).unwrap().is_some());
    drop(reader);
    drop(PimdirStore::open_current(dir.path()).unwrap());
}

#[test]
fn an_absent_store_is_refused_and_not_created() {
    let dir = tempfile::tempdir().unwrap();

    let refused = PimdirStore::open_current(dir.path()).err();
    assert!(
        matches!(
            refused,
            Some(PimdirError::NotCurrent(PimdirNotCurrent::Absent))
        ),
        "{refused:?}"
    );
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        0,
        "nothing written"
    );
}

/// Tables at `user_version` 0 are another program's (§3), as the owner's
/// open reads them; a `user_version` the stamps disagree on is a corrupt
/// store. Neither is migrated.
#[test]
fn another_schema_version_is_refused_and_not_migrated() {
    for (version, code) in [(0, "store-foreign"), (2, "store-corrupt")] {
        let dir = tempfile::tempdir().unwrap();
        drop(PimdirStore::open(dir.path()).unwrap());
        let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
        conn.pragma_update(None, "user_version", version).unwrap();
        drop(conn);

        let refused = PimdirStore::open_current(dir.path()).err().unwrap();
        assert_eq!(refused.code(), code, "{refused:?}");
        let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
        let after: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(after, version, "left as it was");
    }
}

/// Held the way another process would: through a file description this
/// crate's own registry knows nothing about.
#[test]
fn a_store_another_process_owns_is_refused_at_once() {
    let dir = tempfile::tempdir().unwrap();
    drop(PimdirStore::open(dir.path()).unwrap());
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(dir.path().join("owner.lock"))
        .unwrap();
    fs4::FileExt::try_lock(&file).unwrap();

    let refused = PimdirStore::open_current(dir.path()).err();
    assert!(
        matches!(refused, Some(PimdirError::Owned(_))),
        "{refused:?}"
    );

    drop(file);
    drop(PimdirStore::open_current(dir.path()).unwrap());
}

/// A consumer cancelling one queue row owns the store for that write alone:
/// a store of another draft is refused and left as it was, its row still
/// queued (himalaya at 3b4bed8 dropped a newer store's tables by
/// reconciling it).
#[test]
fn a_cancel_leaves_a_store_of_another_draft_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let store = PimdirStore::open(dir.path()).unwrap();
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    drop(store);
    let id = PimdirProducer::open(dir.path(), "test")
        .unwrap()
        .enqueue("INBOX", &PimdirAction::Remove { seq: 1 }, None)
        .unwrap();
    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch(
        "UPDATE store_meta SET format_version = 'storage-draft-11';
         ALTER TABLE task_summary ADD COLUMN newer TEXT;",
    )
    .unwrap();
    drop(conn);
    let before = fs::read(dir.path().join("pimdir.db")).unwrap();

    let refused = PimdirStore::cancel_action(dir.path(), id).err();
    assert!(
        matches!(&refused, Some(PimdirError::Version { found: Some(found) }) if found == "storage-draft-11"),
        "{refused:?}"
    );
    assert_eq!(
        fs::read(dir.path().join("pimdir.db")).unwrap(),
        before,
        "not rebuilt"
    );
    let queued: i64 = rusqlite::Connection::open(dir.path().join("pimdir.db"))
        .unwrap()
        .query_row("SELECT count(*) FROM queue", [], |r| r.get(0))
        .unwrap();
    assert_eq!(queued, 1, "the row stays queued");
}
