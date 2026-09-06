//! What a collection is called against how it is addressed (STORAGE §14).
//!
//! `collections.name` is the label a frontend renders and `collections.id`
//! is the key every foreign key cascades on. An owner namespacing its ids
//! records the bare name, so a reader never splits on a separator that is
//! the owner's own convention, and a DAV collection addressed by a UUID
//! still has something to show.

use io_pimdir::client::{PimdirStore, reader::PimdirReader};

#[test]
fn a_name_moves_without_the_id_it_is_addressed_by() {
    let dir = tempfile::tempdir().unwrap();
    let store = PimdirStore::open(dir.path()).unwrap().for_account("work");

    store
        .ensure_collection("caldav/ED99C7C8", "text/calendar")
        .unwrap();

    let reader = PimdirReader::open(dir.path()).unwrap();
    let seeded = &reader.list_collections().unwrap()[0];
    assert_eq!(
        seeded.name, "caldav/ED99C7C8",
        "with no name of its own a collection is seeded with its id"
    );

    store
        .set_collection_name("caldav/ED99C7C8", "Work")
        .unwrap();

    let named = &reader.list_collections().unwrap()[0];
    assert_eq!(named.name, "Work");
    assert_eq!(named.id, "caldav/ED99C7C8", "the address does not move");
    assert_eq!(named.kind, "text/calendar", "nor the declared kind");
    assert_eq!(named.account.as_deref(), Some("work"));
}

#[test]
fn naming_a_collection_stamps_it_in_the_change_feed() {
    let dir = tempfile::tempdir().unwrap();
    let store = PimdirStore::open(dir.path()).unwrap();
    store
        .ensure_collection("imap/Archives", "message/rfc822")
        .unwrap();

    let reader = PimdirReader::open(dir.path()).unwrap();
    let cursor = reader.change_cursor().unwrap().changed;

    store
        .set_collection_name("imap/Archives", "Archives")
        .unwrap();

    let moved = reader.collections_changed_since(cursor, 10).unwrap();
    assert_eq!(
        moved.len(),
        1,
        "a frontend on the feed is told to re-render"
    );
    assert_eq!(moved[0].id, "imap/Archives");
    assert_eq!(moved[0].name, "Archives");

    let cursor = reader.change_cursor().unwrap().changed;
    store
        .set_collection_name("imap/Archives", "Archives")
        .unwrap();
    assert!(
        reader
            .collections_changed_since(cursor, 10)
            .unwrap()
            .is_empty(),
        "restating the same name moves nothing a reader can observe"
    );
}
