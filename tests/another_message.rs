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

/// A member named in `collection` under `HELD`, its summary stating
/// `subject`, holding `body` when it has one.
fn filed(
    collection: &str,
    handle: &str,
    subject: Option<&str>,
    body: Option<&[u8]>,
) -> Vec<PimdirWriteOp> {
    let PimdirWriteOp::UpsertPlacement(mut placement) =
        named(handle, subject.unwrap_or_default(), "alice@example.org")
    else {
        unreachable!()
    };
    placement.collection = PimdirCollectionId(collection.into());
    if subject.is_none() {
        placement.summary = None;
    }
    let mut ops = Vec::new();
    if let Some(body) = body {
        let object = PimdirObject {
            hash: hash::hash(body),
            size: body.len(),
        };
        placement.object = Some(object.hash.clone());
        placement.level = PimdirLevel::Full;
        placement.base.as_mut().unwrap().object = Some(object.hash.clone());
        ops.push(PimdirWriteOp::StoreObject {
            object,
            body: Some(body.to_vec()),
        });
    }
    ops.push(PimdirWriteOp::UpsertPlacement(placement));
    ops
}

/// A store whose Archive holds `HELD`, the stand-up notes, under
/// `subject`, with `body` when it holds one.
fn archived(
    dir: &std::path::Path,
    subject: Option<&str>,
    body: Option<&[u8]>,
) -> PimdirSourceStore {
    let mut store = PimdirStore::open(dir).unwrap().for_source("imap");
    store
        .ensure_collection("Archive", "message/rfc822")
        .unwrap();
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    store.write(filed("Archive", "5", subject, body)).unwrap();
    store
}

fn seq_in(store: &PimdirSourceStore, collection: &str) -> i64 {
    store
        .seq_for_link(collection, HELD)
        .unwrap()
        .expect("the message has a public id there")
}

/// A message shares the public id of the row holding its `Message-ID` in
/// another mailbox only when it agrees with that row (STORAGE §9, §9.1;
/// vectors/sync/148): another message reusing the id, or a row holding no
/// summary, which agrees with no message, has it draw its own. Before
/// draft-10's second batch the key alone shared the `seq`.
#[test]
fn a_message_shares_a_seq_across_mailboxes_only_with_a_row_it_agrees_with() {
    let dir = tempdir().unwrap();
    let mut store = archived(dir.path(), Some("Stand-up notes"), None);
    store
        .write(filed("INBOX", "20", Some("Stand-up notes"), None))
        .unwrap();
    assert_eq!(
        seq_in(&store, "INBOX"),
        seq_in(&store, "Archive"),
        "one message filed twice is one public id"
    );

    let dir = tempdir().unwrap();
    let mut store = archived(dir.path(), Some("Stand-up notes"), None);
    store
        .write(filed("INBOX", "20", Some("Budget review"), None))
        .unwrap();
    assert_eq!(
        link_of(&store, "20"),
        PimdirLinkId::from(HELD),
        "the key is free in INBOX, so it is taken bare"
    );
    assert_ne!(
        seq_in(&store, "INBOX"),
        seq_in(&store, "Archive"),
        "another message reusing the id is another item"
    );

    let dir = tempdir().unwrap();
    let mut store = archived(dir.path(), None, None);
    store
        .write(filed("INBOX", "20", Some("Stand-up notes"), None))
        .unwrap();
    assert_ne!(
        seq_in(&store, "INBOX"),
        seq_in(&store, "Archive"),
        "a row holding no summary agrees with no message"
    );
}

/// The lookup answers each row of the account holding a body under the
/// key with its size and the subject, date and sender its summary states,
/// the three absent for a row holding none (STORAGE §14, SYNC §6): the
/// witnesses an upgrade links against, one per holder stating another.
#[test]
fn a_lookup_answers_each_holder_with_its_witnesses() {
    let dir = tempdir().unwrap();
    let mut store = archived(dir.path(), Some("Stand-up notes"), Some(b"the notes"));
    store
        .write(filed(
            "INBOX",
            "20",
            Some("Budget review"),
            Some(b"the budget"),
        ))
        .unwrap();
    store.ensure_collection("Sent", "message/rfc822").unwrap();
    store
        .write(filed("Sent", "9", None, Some(b"a third body")))
        .unwrap();

    let found = store.lookup_objects(&[PimdirLinkId::from(HELD)]).unwrap();
    let mut holders: Vec<(Option<String>, usize)> = found[&PimdirLinkId::from(HELD)]
        .iter()
        .map(|held| (held.subject.clone(), held.object.size))
        .collect();
    holders.sort();
    assert_eq!(
        holders,
        [
            (None, 12),
            (Some("Budget review".into()), 10),
            (Some("Stand-up notes".into()), 9),
        ]
    );
    let notes = found[&PimdirLinkId::from(HELD)]
        .iter()
        .find(|held| held.subject.as_deref() == Some("Stand-up notes"))
        .unwrap();
    assert_eq!(notes.sender.as_deref(), Some("alice@example.org"));
    assert_eq!(notes.date.as_deref(), Some("2026-08-01T10:00:00Z"));
    assert_eq!(notes.object.hash, hash::hash(b"the notes"));
}

/// Whether INBOX's row under `HELD` is kept in the trash once its member
/// is gone, the row holding no body.
fn retained_after_the_drop(inbox_subject: Option<&str>, archive_subject: Option<&str>) -> bool {
    let dir = tempdir().unwrap();
    let mut store = archived(dir.path(), archive_subject, None);
    store
        .write(filed("INBOX", "20", inbox_subject, None))
        .unwrap();
    store
        .write(vec![PimdirWriteOp::DropPlacement {
            collection: inbox(),
            handle: PimdirHandle::from("20"),
            reason: PimdirDropReason::Deleted,
        }])
        .unwrap();

    let trash = store.list_retained("INBOX", None, 10).unwrap();
    assert!(trash.len() <= 1);
    assert!(
        store.seq_for_link("Archive", HELD).unwrap().is_some(),
        "the holder is untouched"
    );
    !trash.is_empty()
}

/// A bodiless message losing its last binding is purged at once only for
/// a live row of another mailbox of its account that agrees with it; a
/// holder stating another message, or either row holding no summary,
/// holds nothing of it, and the row is retained (STORAGE §9, §11.1;
/// vectors/sync/149). Before draft-10's second batch any live holder of
/// the key had the row purged.
#[test]
fn a_bodiless_row_is_held_elsewhere_only_by_a_row_that_agrees() {
    assert!(
        !retained_after_the_drop(Some("Stand-up notes"), Some("Stand-up notes")),
        "the message moved, or was filed twice: nothing is lost by the purge"
    );
    assert!(
        retained_after_the_drop(Some("Budget review"), Some("Stand-up notes")),
        "another message reusing the id holds nothing of this one"
    );
    assert!(
        retained_after_the_drop(Some("Stand-up notes"), None),
        "a holder with no summary agrees with no message"
    );
    assert!(
        retained_after_the_drop(None, Some("Stand-up notes")),
        "nor does a retiring row holding none"
    );
}
