//! What a source states a collection is for (STORAGE §14).
//!
//! `collections.role` is written by the owner from the source's own
//! listing (an IMAP special-use attribute, a well-known folder, a default
//! calendar), one holder per role in an account, so a reader learns the
//! sent folder or the default calendar without asking the server.

#![cfg(feature = "client")]

use io_pimdir::client::{PimdirStore, reader::PimdirReader};

fn role_of(reader: &PimdirReader, id: &str) -> Option<String> {
    reader
        .list_collections()
        .unwrap()
        .into_iter()
        .find(|c| c.id == id)
        .unwrap()
        .role
}

#[test]
fn a_role_moves_to_the_collection_it_is_set_on() {
    let dir = tempfile::tempdir().unwrap();
    let store = PimdirStore::open(dir.path()).unwrap().for_account("work");
    store
        .ensure_collection("imap/Sent", "message/rfc822")
        .unwrap();
    store
        .ensure_collection("imap/Sent Items", "message/rfc822")
        .unwrap();

    let reader = PimdirReader::open(dir.path()).unwrap();
    assert_eq!(role_of(&reader, "imap/Sent"), None, "declared with none");

    store
        .set_collection_role("imap/Sent", Some("sent"))
        .unwrap();
    assert_eq!(role_of(&reader, "imap/Sent").as_deref(), Some("sent"));

    let cursor = reader.change_cursor().unwrap().changed;
    store
        .set_collection_role("imap/Sent Items", Some("sent"))
        .unwrap();
    assert_eq!(
        role_of(&reader, "imap/Sent"),
        None,
        "the old holder lost it"
    );
    assert_eq!(role_of(&reader, "imap/Sent Items").as_deref(), Some("sent"));

    let mut moved: Vec<String> = reader
        .collections_changed_since(cursor, 10)
        .unwrap()
        .into_iter()
        .map(|c| c.id)
        .collect();
    moved.sort();
    assert_eq!(moved, ["imap/Sent", "imap/Sent Items"], "both are stamped");

    store.set_collection_role("imap/Sent Items", None).unwrap();
    assert_eq!(role_of(&reader, "imap/Sent Items"), None);
}

#[test]
fn a_role_outside_the_kind_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let store = PimdirStore::open(dir.path()).unwrap();
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    store
        .ensure_collection("caldav/a", "text/calendar")
        .unwrap();

    assert!(store.set_collection_role("INBOX", Some("default")).is_err());
    assert!(store.set_collection_role("caldav/a", Some("sent")).is_err());
    store
        .set_collection_role("caldav/a", Some("default"))
        .unwrap();
}

#[test]
fn each_account_holds_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let root = PimdirStore::open(dir.path()).unwrap();
    let work = root.for_account("work");
    work.ensure_collection("work/INBOX", "message/rfc822")
        .unwrap();
    work.set_collection_role("work/INBOX", Some("inbox"))
        .unwrap();
    drop(work);
    let home = PimdirStore::open(dir.path()).unwrap().for_account("home");
    home.ensure_collection("home/INBOX", "message/rfc822")
        .unwrap();
    home.set_collection_role("home/INBOX", Some("inbox"))
        .unwrap();

    let reader = PimdirReader::open(dir.path()).unwrap();
    assert_eq!(role_of(&reader, "work/INBOX").as_deref(), Some("inbox"));
    assert_eq!(role_of(&reader, "home/INBOX").as_deref(), Some("inbox"));
}
