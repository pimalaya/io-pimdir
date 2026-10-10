//! Files, and the stand-ins of a message's attachments (STORAGE §14.3,
//! Annex A.7).
//!
//! A file is an item of an `application/octet-stream` collection. An
//! attachment is stood for by a file holding no body, keyed by its
//! message and part, tied to the message by an `attachment` reference,
//! and gone once no reference names it.

use std::path::Path;

use io_pimdir::{
    change::{PimdirDropReason, PimdirWriteOp},
    client::{PimdirError, PimdirSourceStore, PimdirStore, reader::PimdirReader},
    collection::PimdirCollectionId,
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
    reference::{PimdirEndpoint, PimdirReferenceOrigin, PimdirReferenceRole},
    summary::{
        PimdirSummary,
        file::{self, PimdirFileSummary, part_key},
    },
};

const MAIL: &str = "message/rfc822";

/// A clean, based placement of `link` under `handle`, holding the body.
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

fn summary(name: &str, part: &str) -> PimdirFileSummary {
    PimdirFileSummary {
        name: name.into(),
        media_type: Some("application/pdf".into()),
        size: Some(1234),
        part: Some(part.into()),
    }
}

/// A store holding message `m` in INBOX, its two attachments stood for
/// in Attachments and referenced from it, and an empty Docs folder.
fn seeded(dir: &Path) -> PimdirSourceStore {
    let mut store = PimdirStore::open(dir).unwrap().for_source("imap");
    store.ensure_collection("INBOX", MAIL).unwrap();
    store.ensure_collection("Attachments", file::KIND).unwrap();
    store.ensure_collection("Docs", file::KIND).unwrap();
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
        ])
        .unwrap();

    let message = PimdirLinkId("m".into());
    for (part, name) in [("2", "Report.pdf"), ("3", "photo.jpg")] {
        let key = part_key(&message, part);
        store
            .put_file("Attachments", &key, &summary(name, part))
            .unwrap();
        store
            .add_reference(
                &PimdirEndpoint {
                    kind: MAIL.into(),
                    link_id: message.clone(),
                },
                &PimdirEndpoint {
                    kind: file::KIND.into(),
                    link_id: key,
                },
                &PimdirReferenceRole::Attachment,
                PimdirReferenceOrigin::Auto,
            )
            .unwrap()
            .unwrap();
    }
    store
}

#[test]
fn a_stand_in_is_a_meta_file_listed_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seeded(dir.path());
    let key = part_key(&PimdirLinkId("m".into()), "2");
    assert_eq!(key.as_str(), "part:m#2");

    let page = store.list_summaries("Attachments", None, 10).unwrap();
    let names: Vec<&str> = page
        .iter()
        .map(|item| match &item.summary {
            Some(PimdirSummary::File(file)) => file.name.as_str(),
            _ => "",
        })
        .collect();
    assert_eq!(names, ["photo.jpg", "Report.pdf"], "A to Z on the name");
    let report = &page[1];
    assert_eq!(report.level, PimdirLevel::Meta);
    assert_eq!(report.object, None, "no body of its own");

    let seq = store
        .put_file("Attachments", &key, &summary("Q3.pdf", "2"))
        .unwrap();
    assert_eq!(seq, report.seq, "restated, not inserted again");
    let item = store.get_item("Attachments", seq).unwrap().unwrap();
    assert_eq!(
        item.summary.map(|s| s.title().to_string()).as_deref(),
        Some("Q3.pdf")
    );

    assert!(matches!(
        store.put_file("INBOX", &key, &summary("x", "2")),
        Err(PimdirError::NotFiles { .. })
    ));
}

#[test]
fn a_message_lists_its_attachments_and_takes_them_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seeded(dir.path());
    let message = PimdirLinkId("m".into());

    // NOTE: saved to Docs, the same key with its own body.
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(placement(
            "Docs", "d1", "part:m#2",
        ))])
        .unwrap();

    let reader = PimdirReader::open(dir.path()).unwrap();
    let attachments = reader.list_attachments(None, &message).unwrap();
    let read: Vec<(&str, &str, Option<&str>, Option<&str>)> = attachments
        .iter()
        .map(|a| {
            (
                a.link_id.as_str(),
                a.collection.as_str(),
                a.summary.as_ref().and_then(|s| s.part.as_deref()),
                a.object.as_ref().map(|o| o.0.as_str()),
            )
        })
        .collect();
    assert_eq!(
        read,
        [
            ("part:m#2", "Attachments", Some("2"), Some("cafebabe")),
            ("part:m#3", "Attachments", Some("3"), None),
        ],
        "in the order recorded, the stand-in read, a saved copy's body held"
    );

    let (m, photo) = (
        PimdirEndpoint {
            kind: MAIL.into(),
            link_id: message.clone(),
        },
        PimdirEndpoint {
            kind: file::KIND.into(),
            link_id: PimdirLinkId("part:m#3".into()),
        },
    );
    store
        .remove_reference(&m, &photo, &PimdirReferenceRole::Attachment)
        .unwrap()
        .unwrap();
    assert_eq!(
        store.seq_for_link("Attachments", "part:m#3").unwrap(),
        None,
        "a stand-in no reference names goes"
    );

    store
        .write(vec![PimdirWriteOp::DropPlacement {
            collection: PimdirCollectionId("INBOX".into()),
            handle: PimdirHandle("1".into()),
            reason: PimdirDropReason::Deleted,
        }])
        .unwrap();
    let seq = store.seq_for_link("INBOX", "m").unwrap().unwrap();
    assert!(store.purge("INBOX", seq).unwrap());
    assert_eq!(
        store.seq_for_link("Attachments", "part:m#2").unwrap(),
        None,
        "the message gone, its stand-in goes"
    );
    assert!(
        store.seq_for_link("Docs", "part:m#2").unwrap().is_some(),
        "the saved copy stays"
    );
}

/// A store an earlier build of draft-04 wrote has no file summaries: a
/// reader lists no attachment, and its owner adds the table and the
/// trigger on open (STORAGE §6).
#[test]
fn an_earlier_store_gains_the_files_on_open() {
    let dir = tempfile::tempdir().unwrap();
    drop(seeded(dir.path()));

    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch(
        "DROP TRIGGER item_reference_collects_files;
         DROP TABLE file_summary;",
    )
    .unwrap();

    let reader = PimdirReader::open(dir.path()).unwrap();
    assert!(
        reader
            .list_attachments(None, &PimdirLinkId("m".into()))
            .unwrap()
            .is_empty()
    );
    drop(reader);

    drop(PimdirStore::open(dir.path()).unwrap());
    let declared: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name IN
                 ('file_summary', 'item_reference_collects_files')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(declared, 2, "the table and its trigger");
}
