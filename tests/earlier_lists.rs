//! A store storage-draft-06 wrote holds a placement's flags, a binding's
//! base flags, a message's `In-Reply-To` ids and a task's categories as
//! JSON arrays, and a pending move names its target in its payload alone.
//! A reader reads it as its reconciliation would leave it before any
//! backfill; its owner reconciles it to the canonical schema on open, every
//! array landing in its table in the same transaction, a `NULL` set staying
//! unknown (STORAGE §6, GUIDE §3).

#![cfg(feature = "client")]

use std::{fs, path::Path};

use io_pimdir::{
    client::{PimdirError, PimdirStore, reader::PimdirReader},
    codec::PimdirAction,
    collection::PimdirCollectionId,
    hub::PimdirSourceId,
    placement::PimdirFlags,
    summary::PimdirSummary,
};
use rusqlite::Connection;

/// migrations/storage/0001_init.sql of storage-draft-06 (pimdir ffe31a4),
/// verbatim.
const DRAFT_06: &str = include_str!("fixtures/storage-draft-06.sql");

/// A draft-06 store: a message with a known flag set, a base and two
/// `In-Reply-To` ids, one whose set and base set are unknown, a task with
/// a known-empty set and two categories, and a move of the second message
/// queued into `Archive`.
fn draft_06_store(flag: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("objects")).unwrap();
    let conn = Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch(DRAFT_06).unwrap();
    conn.execute_batch(&format!(
        r#"INSERT INTO store_meta(id, format, version, hash_algo, created_at, derivation, next_seq)
           VALUES (1, 'pimdir', 1, 'blake3', '2026-10-30T00:00:00.000Z', 2, 4);
         INSERT INTO collections(id, kind, name) VALUES
           ('INBOX', 'message/rfc822', 'INBOX'),
           ('Archive', 'message/rfc822', 'Archive'),
           ('Tasks', 'text/calendar', 'Tasks');
         INSERT INTO items(collection, link_id, seq, flags, sort_key, level) VALUES
           ('INBOX', 'a@example.org', 1, '["$label","{flag}"]', '2026-10-01T00:00:00Z', 1),
           ('INBOX', 'b@example.org', 2, NULL, '2026-10-02T00:00:00Z', 1),
           ('Tasks', 'task-1', 3, '[]', '', 1);
         INSERT INTO bindings(collection, link_id, source, handle, base_flags, base_present) VALUES
           ('INBOX', 'a@example.org', 'imap', '1', '["\\Seen"]', 1),
           ('INBOX', 'b@example.org', 'imap', '2', NULL, 1);
         INSERT INTO mail_summary(collection, link_id, message_id, in_reply_to, subject) VALUES
           ('INBOX', 'a@example.org', 'a@example.org',
            '["root@example.org","parent@example.org"]', 'Re: plan'),
           ('INBOX', 'b@example.org', 'b@example.org', '[]', 'Hello');
         INSERT INTO task_summary(collection, link_id, uid, summary, categories) VALUES
           ('Tasks', 'task-1', 'task-1', 'Call', '["@phone","Errands, small"]');
         INSERT INTO queue(created_at, producer, collection, action, payload) VALUES
           ('2026-10-30T00:00:00.000Z', 'test', 'INBOX', 'move', '{{"v":1,"seq":2,"to":"Archive"}}');
         PRAGMA user_version = 1;"#
    ))
    .unwrap();
    dir
}

fn get(reader: &PimdirReader, collection: &str, seq: i64) -> (PimdirFlags, PimdirSummary) {
    let item = reader.get_item(collection, seq).unwrap().unwrap();
    (item.flags, item.summary.unwrap())
}

fn base(reader: &PimdirReader, link_id: &str) -> Option<PimdirFlags> {
    let bindings = reader.item_bindings("INBOX", link_id).unwrap();
    let binding = &bindings[&PimdirSourceId("imap".into())];
    binding.base.as_ref().map(|base| base.flags.clone())
}

fn in_reply_to(summary: PimdirSummary) -> Vec<String> {
    match summary {
        PimdirSummary::Mail(mail) => mail.in_reply_to,
        other => panic!("a mail summary, got {other:?}"),
    }
}

fn categories(summary: PimdirSummary) -> Vec<String> {
    match summary {
        PimdirSummary::Task(task) => task.categories,
        other => panic!("a task summary, got {other:?}"),
    }
}

fn target(dir: &Path) -> Option<String> {
    Connection::open(dir.join("pimdir.db"))
        .unwrap()
        .query_row("SELECT target_collection FROM queue", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn a_draft_06_store_keeps_its_lists_through_the_reconciliation() {
    let dir = draft_06_store("\\\\Seen");

    // NOTE: unreconciled, the store holds no list table: its sets read as
    // unknown and its lists as none (§6).
    let reader = PimdirReader::open(dir.path()).unwrap();
    let (flags, mail) = get(&reader, "INBOX", 1);
    assert!(flags.is_unknown());
    assert!(in_reply_to(mail).is_empty());
    drop(reader);

    let mut store = PimdirStore::open(dir.path()).unwrap();

    let (flags, mail) = get(&store, "INBOX", 1);
    assert_eq!(flags, PimdirFlags::from_iter(["\\Seen", "$label"]));
    assert_eq!(
        in_reply_to(mail),
        ["root@example.org", "parent@example.org"],
        "the ids in document order"
    );
    assert_eq!(
        base(&store, "a@example.org"),
        Some(PimdirFlags::from_iter(["\\Seen"]))
    );

    let (flags, mail) = get(&store, "INBOX", 2);
    assert!(flags.is_unknown(), "a NULL set stays unknown");
    assert!(in_reply_to(mail).is_empty());
    assert_eq!(base(&store, "b@example.org"), Some(PimdirFlags::Unknown));

    let (flags, task) = get(&store, "Tasks", 3);
    assert_eq!(flags, PimdirFlags::default(), "known-empty stays known");
    assert_eq!(categories(task), ["@phone", "Errands, small"]);

    assert_eq!(target(dir.path()).as_deref(), Some("Archive"));
    store.rename_collection("Archive", "Old").unwrap();
    let pending = store.list_pending_actions().unwrap();
    assert_eq!(
        pending[0].action,
        PimdirAction::Move {
            seq: 2,
            to: PimdirCollectionId("Old".into()),
        },
        "the rename follows the queued move's target"
    );
    drop(store);

    drop(PimdirStore::open_current(dir.path()).unwrap());
}

/// A flag storage-draft-06 held that the store cannot hold any more (§13)
/// refuses the reconciliation, rolled back, rather than losing it.
#[test]
fn a_draft_06_flag_the_store_cannot_hold_refuses_the_reconciliation() {
    let dir = draft_06_store("two words");
    let before = fs::read(dir.path().join("pimdir.db")).unwrap();

    let refused = PimdirStore::open(dir.path()).err().unwrap();
    assert!(
        matches!(&refused, PimdirError::Flag(flag) if flag == "two words"),
        "{refused:?}"
    );
    assert_eq!(refused.code(), "flag-invalid");
    assert_eq!(
        fs::read(dir.path().join("pimdir.db")).unwrap(),
        before,
        "left as it was"
    );
}
