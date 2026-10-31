//! A member stating a `Message-ID` another source's item holds names that
//! item only when its summary agrees on subject, date and sender (SYNC §6,
//! §9): another message reusing the identifier is minted apart, whatever
//! listing named it, so the held item's body never stands for both.

#![cfg(feature = "client")]

use io_pimdir::{
    change::PimdirWriteOp,
    client::{PimdirSourceStore, PimdirStore},
    collection::PimdirCollectionId,
    load::PimdirLoadScope,
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
    summary::{PimdirSummary, mail::PimdirMailSummary},
};
use tempfile::tempdir;

const HELD: &str = "basic-1@example.org";

fn inbox() -> PimdirCollectionId {
    PimdirCollectionId("INBOX".into())
}

/// A member a listing named under `HELD`, its summary stating `subject`
/// and `sender`.
fn named(handle: &str, subject: &str, sender: &str) -> PimdirWriteOp {
    PimdirWriteOp::UpsertPlacement(PimdirPlacement {
        collection: inbox(),
        handle: PimdirHandle::from(handle),
        link_id: Some(PimdirLinkId::from(HELD)),
        object: None,
        level: PimdirLevel::Meta,
        summary: Some(PimdirSummary::Mail(PimdirMailSummary {
            message_id: Some(HELD.into()),
            subject: subject.into(),
            sender: Some(sender.into()),
            date: Some("2026-08-01T10:00:00Z".into()),
            ..Default::default()
        })),
        sort_key: Default::default(),
        flags: PimdirFlags::default(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: None,
            object: None,
        }),
        origin: None,
    })
}

/// A store where source a holds `HELD`, the stand-up notes, and source b.
fn stores(dir: &std::path::Path) -> (PimdirSourceStore, PimdirSourceStore) {
    let mut a = PimdirStore::open(dir).unwrap().for_source("a");
    a.ensure_collection("INBOX", "message/rfc822").unwrap();
    a.write(vec![named("10", "Stand-up notes", "alice@example.org")])
        .unwrap();
    let b = PimdirStore::open(dir).unwrap().for_source("b");
    (a, b)
}

fn link_of(store: &PimdirSourceStore, handle: &str) -> PimdirLinkId {
    store
        .load(&inbox(), &PimdirLoadScope::All)
        .unwrap()
        .placements
        .into_iter()
        .find(|placement| placement.handle.as_str() == handle)
        .and_then(|placement| placement.link_id)
        .expect("a placement under the handle")
}

#[test]
fn another_message_under_a_held_message_id_is_minted_apart() {
    let dir = tempdir().unwrap();
    let (a, mut b) = stores(dir.path());

    b.write(vec![named("77", "Budget review", "dan@example.net")])
        .unwrap();

    assert_eq!(
        link_of(&b, "77"),
        PimdirLinkId::from("dup:basic-1@example.org#77"),
    );
    assert_eq!(link_of(&a, "10"), PimdirLinkId::from(HELD));
    let hub = a.load_hub("INBOX").unwrap();
    let held = &hub.items[&PimdirLinkId::from(HELD)];
    assert_eq!(held.sources.len(), 1, "a's item binds a alone");
    let Some(PimdirSummary::Mail(mail)) = &held.summary else {
        panic!("a mail summary");
    };
    assert_eq!(
        mail.subject, "Stand-up notes",
        "a's message is left as it was"
    );
}

#[test]
fn the_same_message_under_a_held_message_id_joins_it() {
    let dir = tempdir().unwrap();
    let (_, mut b) = stores(dir.path());

    b.write(vec![named("77", "Stand-up notes", "alice@example.org")])
        .unwrap();

    assert_eq!(link_of(&b, "77"), PimdirLinkId::from(HELD));
}
