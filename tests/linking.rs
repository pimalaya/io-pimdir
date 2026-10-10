//! What a linking interface reads (STORAGE §14.1, §14.2): an endpoint
//! described in one read, the per-kind searches, one mail row by key, a
//! sum counting a message once, no reference under a derived key, and the
//! invitation an older store gains on open.

use std::path::Path;

use io_pimdir::{
    change::PimdirWriteOp,
    client::{
        PimdirSourceStore, PimdirStore,
        reader::{PimdirMailFilter, PimdirMailSum, PimdirReader, like_pattern},
    },
    collection::PimdirCollectionId,
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
    reference::{PimdirEndpoint, PimdirReferenceOrigin, PimdirReferenceRole},
    summary::{
        PimdirAddress, PimdirSummary,
        calendar::{PimdirEventSummary, PimdirTaskSummary},
        contact::PimdirContactSummary,
        file::{self, PimdirFileSummary},
        mail::{self, PimdirMailSummary},
    },
};

const MAIL: &str = "message/rfc822";
const CARD: &str = "text/vcard";
const CAL: &str = "text/calendar";

fn endpoint(kind: &str, link_id: &str) -> PimdirEndpoint {
    PimdirEndpoint {
        kind: kind.into(),
        link_id: PimdirLinkId(link_id.into()),
    }
}

/// A clean, based placement of `link` with `summary`, holding `object`.
fn placement(
    collection: &str,
    handle: &str,
    link: &str,
    object: Option<&str>,
    summary: PimdirSummary,
) -> PimdirWriteOp {
    let object = object.map(|hash| PimdirHash(hash.into()));
    PimdirWriteOp::UpsertPlacement(PimdirPlacement {
        sort_key: Default::default(),
        collection: PimdirCollectionId(collection.into()),
        handle: PimdirHandle(handle.into()),
        link_id: Some(PimdirLinkId(link.into())),
        object: object.clone(),
        level: if object.is_some() {
            PimdirLevel::Full
        } else {
            PimdirLevel::Meta
        },
        summary: Some(summary),
        flags: PimdirFlags::default(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: None,
            object,
        }),
        origin: None,
    })
}

fn mail_summary(subject: &str, size: u64) -> PimdirSummary {
    PimdirSummary::Mail(PimdirMailSummary {
        subject: subject.into(),
        size: Some(size),
        ..Default::default()
    })
}

/// A store with a message filed twice (one copy holding its body), a
/// second message, a card, an event, a task and a file.
fn seeded(dir: &Path) -> PimdirSourceStore {
    let mut store = PimdirStore::open(dir).unwrap().for_source("imap");
    for (collection, kind) in [
        ("INBOX", MAIL),
        ("Archive", MAIL),
        ("Cards", CARD),
        ("Cal", CAL),
        ("Docs", file::KIND),
    ] {
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
            placement("INBOX", "1", "m", None, mail_summary("Plan", 10)),
            placement(
                "Archive",
                "1",
                "m",
                Some("cafebabe"),
                mail_summary("Plan", 10),
            ),
            placement("INBOX", "2", "n", None, mail_summary("Lunch", 5)),
            placement(
                "Cards",
                "c",
                "jane",
                None,
                PimdirSummary::Contact(PimdirContactSummary {
                    full_name: "Jane Doe".into(),
                    emails: vec![PimdirAddress {
                        address: "jane@plan.example".into(),
                        name: None,
                    }],
                    ..Default::default()
                }),
            ),
            placement(
                "Cal",
                "e",
                "ev",
                None,
                PimdirSummary::Event(PimdirEventSummary {
                    summary: "Review".into(),
                    location: Some("Plan room".into()),
                    ..Default::default()
                }),
            ),
            placement(
                "Cal",
                "t",
                "todo",
                None,
                PimdirSummary::Task(PimdirTaskSummary {
                    summary: "Send the plan".into(),
                    ..Default::default()
                }),
            ),
            placement(
                "Docs",
                "f",
                "file:01",
                None,
                PimdirSummary::File(PimdirFileSummary {
                    name: "plan.pdf".into(),
                    ..Default::default()
                }),
            ),
        ])
        .unwrap();
    store
}

#[test]
fn a_link_picker_searches_each_kind_and_describes_an_end() {
    let dir = tempfile::tempdir().unwrap();
    drop(seeded(dir.path()));
    let reader = PimdirReader::open(dir.path()).unwrap();
    let plan = like_pattern("plan");

    let contacts = reader.search_contacts(&["Cards"], &plan, None, 10).unwrap();
    assert_eq!(contacts.len(), 1, "matched by its address");
    assert_eq!(contacts[0].item.link_id.as_str(), "jane");
    let calendar = reader.search_calendar(&["Cal"], &plan, None, 10).unwrap();
    assert_eq!(
        calendar
            .iter()
            .map(|hit| (hit.component.as_str(), hit.title.as_str()))
            .collect::<Vec<_>>(),
        [("event", "Review"), ("task", "Send the plan")],
        "a summary, or an event's location"
    );
    let files = reader.search_files(&["Docs"], &plan, None, 10).unwrap();
    assert_eq!(files[0].item.link_id.as_str(), "file:01");

    let view = reader
        .describe_endpoint(&endpoint(CARD, "jane"))
        .unwrap()
        .expect("a live placement");
    assert_eq!(
        (view.collection.as_str(), view.title.as_deref()),
        ("Cards", Some("Jane Doe"))
    );
    assert_eq!(
        reader.describe_endpoint(&endpoint(MAIL, "jane")).unwrap(),
        None,
        "no placement under that kind"
    );
}

#[test]
fn a_message_counts_once_and_reads_by_key() {
    let dir = tempfile::tempdir().unwrap();
    drop(seeded(dir.path()));
    let reader = PimdirReader::open(dir.path()).unwrap();
    let both = ["INBOX", "Archive"];
    let any = PimdirMailFilter::default();

    assert_eq!(
        reader.sum_mail(&both, any, None, None, None).unwrap(),
        PimdirMailSum {
            count: 2,
            size: 15,
            unknown: 0
        },
        "m filed twice counts once"
    );
    assert_eq!(
        reader
            .sum_mail(&both, any, Some(false), None, None)
            .unwrap(),
        PimdirMailSum {
            count: 1,
            size: 5,
            unknown: 0
        },
        "what a download would fetch: m is held in Archive"
    );

    let row = reader
        .get_mail_row("INBOX", &PimdirLinkId("n".into()))
        .unwrap()
        .expect("a live row");
    assert_eq!(
        row.item.summary.map(|s| s.title().to_string()).as_deref(),
        Some("Lunch")
    );
    assert_eq!(
        reader
            .get_mail_row("Cards", &PimdirLinkId("n".into()))
            .unwrap(),
        None
    );
}

#[test]
fn a_derived_key_takes_no_reference() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seeded(dir.path());
    let derived = "alt:Alert|2026-08-01T10:00:00Z|cron@host";
    store
        .write(vec![placement(
            "INBOX",
            "9",
            derived,
            None,
            mail_summary("Alert", 1),
        )])
        .unwrap();
    assert_eq!(
        store
            .add_reference(
                &endpoint(MAIL, derived),
                &endpoint(CARD, "jane"),
                &PimdirReferenceRole::Related,
                PimdirReferenceOrigin::User,
            )
            .unwrap(),
        None
    );
}

/// A store an earlier build wrote has no invitation column: its owner adds
/// it on open, backfills it from the held bodies and records the
/// invitations they imply (STORAGE §6).
#[test]
fn an_earlier_store_backfills_the_invitation_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let body = b"Message-ID: <invite@x>\r\nSubject: Sync\r\nFrom: a@x\r\n\
        Date: Fri, 07 Aug 2026 08:00:00 +0000\r\n\
        Content-Type: text/calendar; method=REQUEST\r\n\r\n\
        BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:ev\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    {
        let mut store = seeded(dir.path());
        let hash = store.blobs().hash(body);
        let summary = mail::derive(body).summary.unwrap();
        store
            .write(vec![
                PimdirWriteOp::StoreObject {
                    object: PimdirObject {
                        hash: hash.clone(),
                        size: body.len(),
                    },
                    body: Some(body.to_vec()),
                },
                placement("INBOX", "3", "invite", Some(hash.as_str()), summary),
            ])
            .unwrap();
    }

    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch(
        "DELETE FROM item_reference;
         DROP INDEX mail_summary_by_invitation;
         ALTER TABLE mail_summary DROP COLUMN invitation;",
    )
    .unwrap();
    drop(conn);

    let store = PimdirStore::open(dir.path()).unwrap();
    let references = store.references_from(&endpoint(MAIL, "invite")).unwrap();
    assert_eq!(
        references
            .iter()
            .map(|r| (r.to.link_id.as_str(), r.role.as_str()))
            .collect::<Vec<_>>(),
        [("ev", "invitation")],
        "backfilled from the held body, then linked"
    );
}
