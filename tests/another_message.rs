//! A member or an `Add` stating a `Message-ID` a row of the collection holds
//! names that row only when its summary agrees on subject, date and sender,
//! a retained row on size too (STORAGE §9; SYNC §6, §9): another message
//! reusing the identifier is minted apart, whatever step named it, so the
//! held row's body never stands for both.

#![cfg(feature = "client")]

use io_pimdir::{
    change::{PimdirDropReason, PimdirWriteOp},
    client::{PimdirSourceStore, PimdirStore},
    collection::PimdirCollectionId,
    hash,
    load::PimdirLoadSelection,
    mutate::PimdirMutation,
    object::PimdirObject,
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
        .load(&inbox(), &PimdirLoadSelection::All)
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

/// The `Add` of a message stating `HELD`, its summary stating `subject`.
fn add(subject: &str) -> PimdirMutation {
    let body = format!("Message-ID: <{HELD}>\r\nSubject: {subject}\r\n\r\nbody\r\n").into_bytes();
    PimdirMutation::Add {
        link_id: PimdirLinkId::from(HELD),
        flags: PimdirFlags::default(),
        object: PimdirObject {
            hash: hash::hash(&body),
            size: body.len(),
        },
        body,
        summary: Some(PimdirSummary::Mail(PimdirMailSummary {
            message_id: Some(HELD.into()),
            subject: subject.into(),
            sender: Some("alice@example.org".into()),
            date: Some("2026-08-01T10:00:00Z".into()),
            ..Default::default()
        })),
        sort_key: Default::default(),
    }
}

/// A store holding `HELD`, the stand-up notes, as a retained row: its
/// only source dropped it.
fn retained(dir: &std::path::Path) -> PimdirSourceStore {
    let (mut a, _) = stores(dir);
    a.write(vec![PimdirWriteOp::DropPlacement {
        collection: inbox(),
        handle: PimdirHandle::from("10"),
        reason: PimdirDropReason::Deleted,
    }])
    .unwrap();
    assert_eq!(a.count_retained(inbox()).unwrap(), 1);
    a
}

/// An `Add` of another message under a retained row's `Message-ID` is
/// minted over the provisional handle it derives, staged under the minted
/// key's own, the retained row left in the trash (STORAGE §9).
#[test]
fn an_add_of_another_message_over_a_retained_one_is_minted_apart() {
    let dir = tempdir().unwrap();
    let mut a = retained(dir.path());

    a.mutate("INBOX", add("Budget review")).unwrap();

    let minted = PimdirLinkId::from("dup:basic-1@example.org#\u{1}basic-1@example.org");
    assert_eq!(link_of(&a, minted.provisional().as_str()), minted);
    assert_eq!(a.count_retained(inbox()).unwrap(), 1, "the trash keeps it");
}

/// The same message revives the retained row (STORAGE §11.1).
#[test]
fn an_add_of_the_same_message_over_a_retained_one_revives_it() {
    let dir = tempdir().unwrap();
    let mut a = retained(dir.path());

    a.mutate("INBOX", add("Stand-up notes")).unwrap();

    let held = PimdirLinkId::from(HELD);
    assert_eq!(link_of(&a, held.provisional().as_str()), held);
    assert_eq!(a.count_retained(inbox()).unwrap(), 0);
}
