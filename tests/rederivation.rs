//! The re-derivation of a store's rows under a later Annex A revision
//! (STORAGE §6): an owner opening a store whose rows an earlier revision
//! wrote re-derives every held message and task from its body, a keyset
//! page at a time, before it records its own revision; the link ids stay
//! as they were made. A store a later revision wrote is left as it stands
//! until this owner's first derived write records its own.

#![cfg(feature = "client")]

use std::path::Path;

use io_pimdir::{
    change::PimdirWriteOp,
    client::PimdirStore,
    collection::PimdirCollectionId,
    hash,
    object::PimdirObject,
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
    summary::mail,
};
use rusqlite::Connection;

/// A message as revision 3 reads it: a comment before its id and before
/// its time, a `Q` word in `iso-8859-1`, a comment inside its sender's
/// name.
fn message(n: usize) -> Vec<u8> {
    format!(
        "Message-ID: (relay) <m{n}@example.org>\r\nSubject: =?iso-8859-1?Q?=80?= {n}\r\n\
From: Night(ops)Shift <ops@example.org>\r\nDate: Sat, 1 Aug 2026 (sent) 10:00:00 +0000\r\n\r\nbody\r\n"
    )
    .into_bytes()
}

/// The ops filing `body` in INBOX under `link`, at `Full`, with what this
/// crate derives from it.
fn file(handle: usize, link: &str, body: &[u8]) -> Vec<PimdirWriteOp> {
    let derivation = mail::derive(body);
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
            sort_key: derivation.sort_key,
            collection: PimdirCollectionId("INBOX".into()),
            handle: PimdirHandle(handle.to_string()),
            link_id: Some(PimdirLinkId(link.into())),
            object: Some(hash.clone()),
            level: PimdirLevel::Full,
            summary: derivation.summary,
            flags: PimdirFlags::default(),
            status: PimdirStatus::Clean,
            conflict_revision: None,
            conflict_object: None,
            base: Some(PimdirBase {
                flags: PimdirFlags::default(),
                revision: None,
                object: Some(hash),
            }),
            origin: None,
        }),
    ]
}

/// A task due in a zone its resource defines no `VTIMEZONE` for.
const TASK: &[u8] = b"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:t1\r\n\
SUMMARY:File the lease\r\nDUE;TZID=Europe/Paris:20260814T090000\r\nEND:VTODO\r\n\
END:VCALENDAR\r\n";

/// A store holding 300 messages, one keyed `(relay) <m0@example.org>`,
/// the field's text an earlier reading made the key of, and a task.
fn store() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap();
    store.ensure_collection("Tasks", "text/calendar").unwrap();
    store.put_item("Tasks", TASK).unwrap();

    let mut store = store.for_source("imap");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    let mut ops = file(0, "(relay) <m0@example.org>", &message(0));
    for n in 1..300 {
        ops.extend(file(n, &format!("m{n}@example.org"), &message(n)));
    }
    store.write(ops).unwrap();
    dir
}

fn db(dir: &Path) -> Connection {
    Connection::open(dir.join("pimdir.db")).unwrap()
}

/// Rewrites the derived rows as an earlier revision would have left
/// them, all but `m299`'s, and records `revision`.
fn as_written_under(dir: &Path, revision: i64) {
    db(dir)
        .execute_batch(&format!(
            "UPDATE mail_summary SET subject = 'stale', date = NULL, sender_name = 'stale'
             WHERE link_id <> 'm299@example.org';
             UPDATE item_address SET name = 'stale' WHERE link_id <> 'm299@example.org';
             UPDATE items SET sort_key = '' WHERE link_id <> 'm299@example.org';
             UPDATE store_meta SET derivation = {revision};"
        ))
        .unwrap();
}

/// What a message's row and its sender's address hold.
fn mail_row(dir: &Path, link: &str) -> (String, Option<String>, Option<String>, String, String) {
    db(dir)
        .query_row(
            "SELECT m.subject, m.date, m.sender_name, a.name, i.sort_key
             FROM mail_summary m
             JOIN items i ON i.collection = m.collection AND i.link_id = m.link_id
             JOIN item_address a ON a.collection = m.collection AND a.link_id = m.link_id
             WHERE m.link_id = ?1 AND a.role = 'from'",
            [link],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap()
}

fn changed(dir: &Path, link: &str) -> i64 {
    db(dir)
        .query_row(
            "SELECT changed FROM items WHERE link_id = ?1",
            [link],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn an_owner_re_derives_a_store_an_earlier_revision_wrote() {
    for revision in [1, 2] {
        let dir = store();
        as_written_under(dir.path(), revision);
        let untouched = changed(dir.path(), "m299@example.org");
        let stamped = changed(dir.path(), "m5@example.org");

        let store = PimdirStore::open(dir.path()).unwrap();
        assert_eq!(store.store_meta().unwrap().derivation, 3, "from {revision}");
        drop(store);

        let fresh = (
            "\u{20ac} 5".to_string(),
            Some("2026-08-01T10:00:00Z".to_string()),
            Some("Night Shift".to_string()),
            "Night Shift".to_string(),
            "2026-08-01T10:00:00Z".to_string(),
        );
        assert_eq!(mail_row(dir.path(), "m5@example.org"), fresh);
        let stale: i64 = db(dir.path())
            .query_row(
                "SELECT count(*) FROM mail_summary WHERE subject = 'stale'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stale, 0, "every page, past the first 256 keys");
        assert!(changed(dir.path(), "m5@example.org") > stamped, "stamped");
        assert_eq!(
            changed(dir.path(), "m299@example.org"),
            untouched,
            "a row that did not move draws no stamp"
        );

        let (key, id): (String, String) = db(dir.path())
            .query_row(
                "SELECT link_id, message_id FROM mail_summary WHERE subject = '\u{20ac} 0'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(key, "(relay) <m0@example.org>", "the link id kept");
        assert_eq!(id, "m0@example.org", "the column re-derived");

        let task: String = db(dir.path())
            .query_row(
                "SELECT sort_key FROM items WHERE link_id = 't1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            task, "2026-08-14T07:00:00Z",
            "the task through the database"
        );
    }
}

/// An owner opening a store a later revision wrote leaves its rows and
/// its revision; its first derived write records its own (§6).
#[test]
fn an_owner_leaves_a_store_a_later_revision_wrote() {
    let dir = store();
    as_written_under(dir.path(), 4);

    let mut store = PimdirStore::open(dir.path()).unwrap();
    assert_eq!(store.store_meta().unwrap().derivation, 4);
    assert_eq!(mail_row(dir.path(), "m5@example.org").0, "stale");

    let card = b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:urn:uuid:a\r\nFN:A\r\nEND:VCARD\r\n";
    store.ensure_collection("Contacts", "text/vcard").unwrap();
    store.put_item("Contacts", card).unwrap();
    assert_eq!(store.store_meta().unwrap().derivation, 3);
}
