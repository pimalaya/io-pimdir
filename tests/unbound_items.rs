//! Items a person keeps on the device, in a collection no source syncs
//! (STORAGE §14.3): the local time collection's entries, and any card or
//! calendar item written there.
//!
//! The owner derives each item's key and summary from its body as a synced
//! write does, writes the item, its summary, its references and its pins in
//! one transaction, and refuses a collection a source syncs, whose items
//! are that source's.

#![cfg(feature = "client")]

use io_pimdir::{
    change::PimdirWriteOp,
    client::{PimdirError, PimdirStore, TIME_COLLECTION},
    collection::PimdirCollectionId,
    object::PimdirObject,
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
    summary::PimdirSummary,
};

/// A time entry tied to a task, running when `end` is `None` (pimdir
/// GUIDE.md §18).
fn entry(end: Option<&str>) -> Vec<u8> {
    let end = end
        .map(|end| format!("DTEND:{end}\r\n"))
        .unwrap_or_default();
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//test//EN\r\nBEGIN:VEVENT\r\n\
         UID:entry-1@example.org\r\nDTSTAMP:20261028T090000Z\r\n\
         DTSTART:20261028T090000Z\r\n{end}SUMMARY:Paint the hallway\r\n\
         TRANSP:TRANSPARENT\r\nRELATED-TO:todo-next@example.org\r\n\
         END:VEVENT\r\nEND:VCALENDAR\r\n"
    )
    .into_bytes()
}

fn dtend(store: &PimdirStore, seq: i64) -> Option<String> {
    match store
        .get_item(TIME_COLLECTION, seq)
        .unwrap()
        .unwrap()
        .summary
    {
        Some(PimdirSummary::Event(event)) => event.dtend,
        other => panic!("an event summary, got {other:?}"),
    }
}

#[test]
fn the_time_collection_is_a_calendar_under_no_account_no_source_syncs() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap();

    assert_eq!(store.ensure_time_collection().unwrap(), "\u{1}time");
    store.ensure_time_collection().unwrap();

    let collections = store.list_collections_by_account(None).unwrap();
    assert_eq!(collections.len(), 1);
    assert_eq!(collections[0].id, TIME_COLLECTION);
    assert_eq!(collections[0].kind, "text/calendar");
    assert_eq!(collections[0].name, "time");
    assert!(
        store
            .collection_sources(TIME_COLLECTION)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn an_entry_is_written_restated_and_deleted_with_its_pins() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap();
    store.ensure_time_collection().unwrap();

    let (link, seq) = store.put_item(TIME_COLLECTION, &entry(None)).unwrap();
    assert_eq!(link, PimdirLinkId("entry-1@example.org".into()));
    assert_eq!(dtend(&store, seq), None, "a running entry has no end");
    let running = store.get_item(TIME_COLLECTION, seq).unwrap().unwrap();
    assert_eq!(running.level, PimdirLevel::Full);
    assert_eq!(running.object, Some(store.hash(&entry(None))));
    assert!(store.blobs().path(&store.hash(&entry(None))).is_file());

    let (_, restated) = store
        .put_item(TIME_COLLECTION, &entry(Some("20261028T103000Z")))
        .unwrap();
    assert_eq!(restated, seq, "the same key keeps its public id");
    assert_eq!(dtend(&store, seq).as_deref(), Some("20261028T103000Z"));
    assert_eq!(store.count_items(TIME_COLLECTION).unwrap(), 1);
    assert!(store.refcount_drift().unwrap().is_empty());

    assert!(store.delete_unbound(TIME_COLLECTION, &link).unwrap());
    assert!(store.get_item(TIME_COLLECTION, seq).unwrap().is_none());
    assert!(store.refcount_drift().unwrap().is_empty());
}

#[test]
fn a_collection_a_source_syncs_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut source = PimdirStore::open(dir.path()).unwrap().for_source("caldav");
    source
        .ensure_collection("Calendar", "text/calendar")
        .unwrap();
    let body = entry(None);
    let hash = source.hash(&body);
    source
        .write(vec![
            PimdirWriteOp::StoreObject {
                object: PimdirObject {
                    hash: hash.clone(),
                    size: body.len(),
                },
                body: Some(body.clone()),
            },
            PimdirWriteOp::UpsertPlacement(PimdirPlacement {
                sort_key: Default::default(),
                collection: PimdirCollectionId("Calendar".into()),
                handle: PimdirHandle("other.ics".into()),
                link_id: Some(PimdirLinkId("other@example.org".into())),
                object: Some(hash.clone()),
                level: PimdirLevel::Full,
                summary: None,
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
        ])
        .unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap();

    assert!(matches!(
        store.put_item("Calendar", &body),
        Err(PimdirError::Synced { collection }) if collection == "Calendar"
    ));
    assert_eq!(store.count_items("Calendar").unwrap(), 1);
}

#[test]
fn mail_files_and_undeclared_collections_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap();
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    store
        .ensure_collection("Docs", "application/octet-stream")
        .unwrap();

    for collection in ["INBOX", "Docs", "Nowhere"] {
        assert!(
            matches!(
                store.put_item(collection, &entry(None)),
                Err(PimdirError::NotUnbound { .. })
            ),
            "{collection} refused"
        );
    }
}

#[test]
fn a_card_kept_on_the_device_is_written_with_its_summary() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap();
    store.ensure_collection("Local", "text/vcard").unwrap();
    let card =
        b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:urn:uuid:local-1\r\nFN:Jo Plumber\r\nEND:VCARD\r\n";

    let (link, seq) = store.put_item("Local", card).unwrap();
    assert_eq!(link, PimdirLinkId("urn:uuid:local-1".into()));
    match store.get_item("Local", seq).unwrap().unwrap().summary {
        Some(PimdirSummary::Contact(contact)) => assert_eq!(contact.full_name, "Jo Plumber"),
        other => panic!("a contact summary, got {other:?}"),
    }
}

/// A queued action on an item no source binds is the store's to settle
/// (§15.2): never skipped, which would leave it pending forever. An edit
/// and a flag change of a device-only item apply to the store, a remove
/// retains it, and a move, which nothing could push, parks.
#[test]
fn a_queued_action_on_an_item_no_source_binds_applies_or_parks() {
    use io_pimdir::{client::producer::PimdirProducer, codec::PimdirAction};

    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap();
    store.ensure_time_collection().unwrap();
    store.ensure_collection("archive", "text/calendar").unwrap();
    let (_, seq) = store.put_item(TIME_COLLECTION, &entry(None)).unwrap();
    drop(store);

    let mut producer = PimdirProducer::open(dir.path(), "app").unwrap();
    producer
        .enqueue(
            TIME_COLLECTION,
            &PimdirAction::SetFlags {
                seq,
                flags: PimdirFlags::from_iter(["\\Flagged"]),
            },
            None,
        )
        .unwrap();
    producer
        .enqueue_with_body(TIME_COLLECTION, &entry(Some("20261028T100000Z")), |hash| {
            PimdirAction::Update {
                seq,
                object: hash,
                if_match: None,
            }
        })
        .unwrap();
    drop(producer);

    let mut owner = PimdirStore::open(dir.path()).unwrap().for_source("phone");
    let report = owner.drain().unwrap();
    assert_eq!(
        (report.applied, report.parked, report.skipped),
        (2, 0, 0),
        "{report:?}"
    );
    assert_eq!(dtend(&owner, seq).as_deref(), Some("20261028T100000Z"));
    let item = owner.get_item(TIME_COLLECTION, seq).unwrap().unwrap();
    assert!(item.flags.contains("\\Flagged"), "{:?}", item.flags);
    drop(owner);

    let mut producer = PimdirProducer::open(dir.path(), "app").unwrap();
    producer
        .enqueue(
            TIME_COLLECTION,
            &PimdirAction::Move {
                seq,
                to: PimdirCollectionId("archive".into()),
            },
            None,
        )
        .unwrap();
    producer
        .enqueue(TIME_COLLECTION, &PimdirAction::Remove { seq }, None)
        .unwrap();
    drop(producer);

    let mut owner = PimdirStore::open(dir.path()).unwrap().for_source("phone");
    let report = owner.drain().unwrap();
    assert_eq!(
        (report.applied, report.parked, report.skipped),
        (1, 1, 0),
        "{report:?}"
    );
    assert!(owner.get_item(TIME_COLLECTION, seq).unwrap().is_none());
    assert_eq!(
        owner
            .list_retained(TIME_COLLECTION, None, 10)
            .unwrap()
            .len(),
        1
    );
    assert!(owner.list_pending_actions().unwrap().is_empty());
}

/// An edit of an item no source binds applies on an edit's usual terms
/// (STORAGE §15.3), `if_match` included: one made over a body another
/// write has replaced since parks naming both, the item left as it is.
#[test]
fn an_unbound_edit_over_a_replaced_body_parks() {
    use io_pimdir::{client::producer::PimdirProducer, codec::PimdirAction};

    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap();
    store.ensure_time_collection().unwrap();
    let (_, seq) = store.put_item(TIME_COLLECTION, &entry(None)).unwrap();
    let read = store.hash(&entry(None));
    store
        .put_item(TIME_COLLECTION, &entry(Some("20261028T110000Z")))
        .unwrap();
    drop(store);

    let mut producer = PimdirProducer::open(dir.path(), "app").unwrap();
    producer
        .enqueue_with_body(TIME_COLLECTION, &entry(Some("20261028T100000Z")), |hash| {
            PimdirAction::Update {
                seq,
                object: hash,
                if_match: Some(read),
            }
        })
        .unwrap();
    drop(producer);

    let mut owner = PimdirStore::open(dir.path()).unwrap().for_source("phone");
    let report = owner.drain().unwrap();
    assert_eq!(
        (report.applied, report.parked, report.skipped),
        (0, 1, 0),
        "{report:?}"
    );
    assert_eq!(dtend(&owner, seq).as_deref(), Some("20261028T110000Z"));
    let parked = owner.parked_actions().unwrap();
    assert!(
        parked[0].error.contains("body replaced since it was read"),
        "{parked:?}"
    );
}
