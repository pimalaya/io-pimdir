//! The public id and what hangs on it (STORAGE §4.3, §9.1), ported from
//! pimdir's retired checks/invariants.sh: one identity keeps one seq
//! within its kind, and an item's properties follow its seq.

#![cfg(feature = "client")]

mod invariants_common;

use io_pimdir::{change::PimdirWriteOp, client::PimdirSourceStore, summary::PimdirSummary};

use invariants_common::*;

const AUG: &str = "Sat, 1 Aug 2026 10:00:00 +0000";

fn seq(store: &PimdirSourceStore, collection: &str, link: &str) -> i64 {
    store.seq_for_link(collection, link).unwrap().unwrap()
}

/// L949: a Message-ID spelling an event's UID is another item, drawing its
/// own seq.
#[test]
fn l949_a_message_id_spelling_an_event_uid_draws_its_own_seq() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    store.ensure_collection("Cal", "text/calendar").unwrap();
    let event = b"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//x//x//EN\r\n\
        BEGIN:VEVENT\r\nUID:x@example.org\r\nDTSTAMP:20260801T100000Z\r\n\
        DTSTART:20260801T100000Z\r\nSUMMARY:E\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let (link, event_seq) = store.put_item("Cal", event).unwrap();
    assert_eq!(link.as_str(), "x@example.org");

    let mail = message("x@example.org", "", AUG, None);
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(meta(
            "INBOX", "1", &mail,
        ))])
        .unwrap();
    assert_ne!(seq(&store, "INBOX", "x@example.org"), event_seq);
}

/// L966: an unknown date and sender agree with unknown ones, and the size
/// plays no part in the agreement.
#[test]
fn l966_unknown_date_and_sender_agree_whatever_the_size() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    let bare = message("bare@x", "", "someday", None);
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(meta(
            "Archive", "1", &bare,
        ))])
        .unwrap();
    let mut other = meta("INBOX", "1", &bare);
    match other.summary.as_mut() {
        Some(PimdirSummary::Mail(mail)) => mail.size = Some(9_999),
        _ => unreachable!(),
    }
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(other)])
        .unwrap();

    assert_eq!(
        seq(&store, "INBOX", "bare@x"),
        seq(&store, "Archive", "bare@x")
    );
}

/// L975: of several rows that agree with a member, the lowest seq is
/// shared.
#[test]
fn l975_of_several_agreeing_rows_the_lowest_seq_is_shared() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    store.ensure_collection("Sent", MAIL).unwrap();
    let notice = message("m@x", "Notice", AUG, Some("cron@x"));
    let other = message("m@x", "Other", AUG, Some("cron@x"));
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(meta(
            "INBOX", "1", &notice,
        ))])
        .unwrap();
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(meta(
            "Archive", "1", &other,
        ))])
        .unwrap();
    let low = seq(&store, "INBOX", "m@x");
    let high = seq(&store, "Archive", "m@x");
    assert_ne!(low, high, "a disagreeing summary draws its own");
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(meta(
            "Archive", "1", &notice,
        ))])
        .unwrap();
    assert_eq!(
        seq(&store, "Archive", "m@x"),
        high,
        "restated under its seq"
    );

    store
        .write(vec![PimdirWriteOp::UpsertPlacement(meta(
            "Sent", "1", &notice,
        ))])
        .unwrap();
    assert_eq!(seq(&store, "Sent", "m@x"), low.min(high));
}

/// L4156: a retained item revived comes back under its seq, its
/// properties with it.
#[test]
fn l4156_a_revive_restores_the_seq_and_its_properties() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    let a = message("a@x", "A", AUG, Some("alice@x"));
    store
        .write(vec![
            body(&store, &a),
            PimdirWriteOp::UpsertPlacement(held(&store, "INBOX", "1", &a)),
        ])
        .unwrap();
    let before = seq(&store, "INBOX", "a@x");
    assert!(store.add_property(before, "app/label", "work").unwrap());
    store.write(vec![deleted("INBOX", "1")]).unwrap();
    assert_eq!(store.count_retained("INBOX").unwrap(), 1);

    store
        .write(vec![PimdirWriteOp::UpsertPlacement(held(
            &store, "INBOX", "1", &a,
        ))])
        .unwrap();

    assert_eq!(store.count_retained("INBOX").unwrap(), 0, "revived");
    assert_eq!(seq(&store, "INBOX", "a@x"), before);
    let properties = store.list_item_properties(before).unwrap();
    assert_eq!(properties.len(), 1);
    assert_eq!(
        (properties[0].key.as_str(), properties[0].value.as_str()),
        ("app/label", "work")
    );
}

/// L4165: a collection's delete takes the properties of the seqs whose
/// last row it held, and keeps those of a seq held elsewhere.
#[test]
fn l4165_a_collection_delete_takes_the_properties_of_its_last_rows() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    let a = message("a@x", "A", AUG, Some("alice@x"));
    let b = message("b@x", "B", AUG, Some("alice@x"));
    store
        .write(vec![
            PimdirWriteOp::UpsertPlacement(meta("Archive", "1", &a)),
            PimdirWriteOp::UpsertPlacement(meta("Archive", "2", &b)),
        ])
        .unwrap();
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(meta("INBOX", "2", &b))])
        .unwrap();
    let only = seq(&store, "Archive", "a@x");
    let shared = seq(&store, "Archive", "b@x");
    assert_eq!(seq(&store, "INBOX", "b@x"), shared);
    assert!(store.add_property(only, "app/label", "x").unwrap());
    assert!(store.add_property(shared, "app/label", "y").unwrap());

    assert!(store.delete_collection("Archive").unwrap());

    assert!(store.list_item_properties(only).unwrap().is_empty());
    assert_eq!(store.list_item_properties(shared).unwrap().len(), 1);
}
