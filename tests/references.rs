//! References between items (STORAGE §14.2).
//!
//! A reference names each end by its kind and link id, so it holds while
//! any row of either end remains, live, tombstoned or retained, and goes
//! with the last. Recording one again changes nothing, save a person's
//! taking over a rule's.

use std::path::Path;

use io_pimdir::{
    change::{PimdirDropReason, PimdirWriteOp},
    client::{PimdirSourceStore, PimdirStore, reader::PimdirReader},
    collection::PimdirCollectionId,
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
    reference::{PimdirEndpoint, PimdirReferenceOrigin, PimdirReferenceRole},
    summary::{
        PimdirAddress, PimdirSummary, calendar::PimdirEventSummary, contact::PimdirContactSummary,
        mail::PimdirMailSummary,
    },
};

const MAIL: &str = "message/rfc822";
const CARD: &str = "text/vcard";

fn endpoint(kind: &str, link_id: &str) -> PimdirEndpoint {
    PimdirEndpoint {
        kind: kind.into(),
        link_id: PimdirLinkId(link_id.into()),
    }
}

/// A clean, based placement of `link` under `handle`, holding the one body.
fn placement(collection: &str, handle: &str, link: &str) -> PimdirPlacement {
    PimdirPlacement {
        sort_key: Default::default(),
        collection: PimdirCollectionId(collection.into()),
        handle: PimdirHandle(handle.into()),
        link_id: Some(PimdirLinkId(link.into())),
        object: Some(PimdirHash("cafebabe".into())),
        level: PimdirLevel::Full,
        summary: None,
        flags: PimdirFlags::default(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: None,
            object: Some(PimdirHash("cafebabe".into())),
        }),
        origin: None,
    }
}

/// A store holding `m` in INBOX and Archive, `n` in INBOX and `alice` in
/// Cards.
fn seeded(dir: &Path) -> PimdirSourceStore {
    let mut store = PimdirStore::open(dir).unwrap().for_source("imap");
    for (collection, kind) in [("INBOX", MAIL), ("Archive", MAIL), ("Cards", CARD)] {
        store.ensure_collection(collection, kind).unwrap();
    }
    store
        .write(vec![
            PimdirWriteOp::StoreObject {
                object: PimdirObject {
                    hash: PimdirHash("cafebabe".into()),
                    size: 4,
                },
                body: Some(b"body".to_vec()),
            },
            PimdirWriteOp::UpsertPlacement(placement("INBOX", "1", "m")),
            PimdirWriteOp::UpsertPlacement(placement("INBOX", "2", "n")),
            PimdirWriteOp::UpsertPlacement(placement("Archive", "1", "m")),
            PimdirWriteOp::UpsertPlacement(placement("Cards", "a", "alice")),
        ])
        .unwrap();
    store
}

#[test]
fn a_reference_is_recorded_once_and_read_from_either_end() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let (m, alice) = (endpoint(MAIL, "m"), endpoint(CARD, "alice"));
    let sender = PimdirReferenceRole::Sender;

    let recorded = store
        .add_reference(&m, &alice, &sender, PimdirReferenceOrigin::Auto)
        .unwrap()
        .expect("recorded");
    assert_eq!(recorded.origin, PimdirReferenceOrigin::Auto);
    assert!(recorded.created_at.ends_with('Z'));
    assert_eq!(
        store
            .add_reference(&m, &alice, &sender, PimdirReferenceOrigin::Auto)
            .unwrap(),
        Some(recorded.clone()),
        "a duplicate records nothing and answers the reference standing"
    );
    let taken = store
        .add_reference(&m, &alice, &sender, PimdirReferenceOrigin::User)
        .unwrap()
        .expect("a person's takes over");
    assert_eq!(taken.origin, PimdirReferenceOrigin::User);
    assert_eq!(
        store
            .add_reference(&m, &alice, &sender, PimdirReferenceOrigin::Auto)
            .unwrap(),
        Some(taken.clone()),
        "a rule's never takes back"
    );

    let reader = PimdirReader::open(dir.path()).unwrap();
    let read = reader.references_from(&m).unwrap();
    assert_eq!(read, reader.references_to(&alice).unwrap());
    assert_eq!(read, [taken]);
    assert!(reader.references_to(&m).unwrap().is_empty());

    let thread = PimdirReferenceRole::Application("x-moa-thread".into());
    let n = endpoint(MAIL, "n");
    let linked = store
        .add_reference(&m, &n, &thread, PimdirReferenceOrigin::User)
        .unwrap()
        .expect("an application's role");
    assert_eq!(
        PimdirReferenceRole::parse(linked.role.as_str()),
        thread,
        "read back as the application's"
    );
    let unknown = PimdirReferenceRole::Application("friend".into());
    assert!(
        store
            .add_reference(&m, &n, &unknown, PimdirReferenceOrigin::User)
            .is_err(),
        "a name without x- is refused"
    );
    assert!(
        store
            .add_reference(&m, &m, &thread, PimdirReferenceOrigin::User)
            .is_err(),
        "an item never refers to itself"
    );
    assert_eq!(
        store
            .add_reference(
                &m,
                &endpoint(CARD, "ghost"),
                &sender,
                PimdirReferenceOrigin::User
            )
            .unwrap(),
        None,
        "an end the store holds no row of"
    );
    assert_eq!(
        store
            .add_reference(
                &m,
                &endpoint(MAIL, "alice"),
                &sender,
                PimdirReferenceOrigin::User
            )
            .unwrap(),
        None,
        "nor one of another kind"
    );

    assert!(store.remove_reference(&m, &n, &thread).unwrap().is_some());
    assert_eq!(store.remove_reference(&m, &n, &thread).unwrap(), None);
}

#[test]
fn a_reference_goes_with_the_last_row_of_an_end() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seeded(dir.path());
    let (m, n, alice) = (
        endpoint(MAIL, "m"),
        endpoint(MAIL, "n"),
        endpoint(CARD, "alice"),
    );
    let related = PimdirReferenceRole::Related;
    store
        .add_reference(&n, &m, &related, PimdirReferenceOrigin::User)
        .unwrap()
        .unwrap();
    store
        .add_reference(&n, &alice, &related, PimdirReferenceOrigin::User)
        .unwrap()
        .unwrap();

    let drop = |collection: &str, handle: &str| PimdirWriteOp::DropPlacement {
        collection: PimdirCollectionId(collection.into()),
        handle: PimdirHandle(handle.into()),
        reason: PimdirDropReason::Deleted,
    };
    store.write(vec![drop("INBOX", "1")]).unwrap();
    assert_eq!(
        store.references_to(&m).unwrap().len(),
        1,
        "Archive still holds m"
    );

    store.write(vec![drop("Archive", "1")]).unwrap();
    let retained = store.list_retained(PimdirCollectionId("Archive".into()), None, 10);
    let seq = retained.unwrap()[0].seq;
    assert_eq!(
        store.references_to(&m).unwrap().len(),
        1,
        "a retained last row still holds it"
    );
    assert!(store.purge("Archive", seq).unwrap());
    assert!(
        store.references_to(&m).unwrap().is_empty(),
        "the purge of the last row takes it"
    );

    assert!(store.delete_collection("Cards").unwrap());
    assert!(
        store.references_from(&n).unwrap().is_empty(),
        "a collection's delete takes the references of what it held last"
    );
}

/// A store an earlier build of draft-04 wrote has no references and no
/// invitation: a reader reads none, and its owner adds the table, its
/// index, its trigger and the column on open (STORAGE §6).
#[test]
fn an_earlier_store_gains_the_references_on_open() {
    let dir = tempfile::tempdir().unwrap();
    drop(seeded(dir.path()));

    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch(
        "DROP TRIGGER items_drop_references;
         DROP INDEX item_reference_to;
         DROP TABLE item_reference;
         DROP INDEX mail_summary_by_invitation;
         ALTER TABLE mail_summary DROP COLUMN invitation;",
    )
    .unwrap();

    let reader = PimdirReader::open(dir.path()).unwrap();
    assert!(
        reader
            .references_from(&endpoint(MAIL, "m"))
            .unwrap()
            .is_empty()
    );
    drop(reader);

    let store = PimdirStore::open(dir.path()).unwrap();
    let declared: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name IN
                 ('item_reference', 'item_reference_to', 'items_drop_references',
                  'mail_summary_by_invitation')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        declared, 4,
        "the table, its index, its trigger, the invitation's index"
    );
    let invitation: i64 = conn
        .query_row(
            "SELECT count(*) FROM pragma_table_info('mail_summary') WHERE name = 'invitation'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(invitation, 1, "the invitation column");
    assert!(
        store
            .add_reference(
                &endpoint(MAIL, "m"),
                &endpoint(CARD, "alice"),
                &PimdirReferenceRole::Sender,
                PimdirReferenceOrigin::Auto,
            )
            .unwrap()
            .is_some()
    );
}

/// The writer records the automatic references a written summary reads
/// (STORAGE §14.2), from the mail to what it names, whichever end lands
/// first: a contact before or after the mail it sent, an event after its
/// invitation. A person's reference is never touched.
#[test]
fn the_writer_records_the_automatic_references() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seeded(dir.path());
    store.ensure_collection("Cal", "text/calendar").unwrap();
    let alice = || PimdirAddress {
        address: "alice@example.org".into(),
        name: None,
    };
    let summarised = |collection: &str, handle: &str, link: &str, summary: PimdirSummary| {
        PimdirWriteOp::UpsertPlacement(PimdirPlacement {
            summary: Some(summary),
            ..placement(collection, handle, link)
        })
    };
    let card = |name: &str| {
        PimdirSummary::Contact(PimdirContactSummary {
            full_name: name.into(),
            emails: vec![alice()],
            ..Default::default()
        })
    };

    store
        .write(vec![summarised("Cards", "b", "bob", card("Bob"))])
        .unwrap();
    store
        .write(vec![summarised(
            "INBOX",
            "9",
            "invite",
            PimdirSummary::Mail(PimdirMailSummary {
                subject: "Weekly sync".into(),
                invitation: Some("ev1".into()),
                from: vec![alice()],
                ..Default::default()
            }),
        )])
        .unwrap();
    store
        .write(vec![summarised("Cards", "c", "carol", card("Carol"))])
        .unwrap();
    store
        .write(vec![summarised(
            "Cal",
            "e",
            "ev1",
            PimdirSummary::Event(PimdirEventSummary {
                uid: Some("ev1".into()),
                summary: "Weekly sync".into(),
                ..Default::default()
            }),
        )])
        .unwrap();

    let read: Vec<(String, String, PimdirReferenceOrigin)> = store
        .references_from(&endpoint(MAIL, "invite"))
        .unwrap()
        .into_iter()
        .map(|r| (r.to.link_id.0, r.role.as_str().to_string(), r.origin))
        .collect();
    let auto = PimdirReferenceOrigin::Auto;
    assert_eq!(
        read,
        [
            ("bob".into(), "sender".into(), auto),
            ("carol".into(), "sender".into(), auto),
            ("ev1".into(), "invitation".into(), auto),
        ],
        "the sender before and after, the event after its invitation"
    );
}
