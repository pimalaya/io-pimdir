//! References between items and what a summary keeps (STORAGE §14.2,
//! Annex A.1), ported from pimdir's retired checks/invariants.sh: a
//! reference's end is a kind and a key, a stand-in lives while a
//! reference names it, and a bodiless write keeps a known invitation.

#![cfg(feature = "client")]

mod invariants_common;

use io_pimdir::{
    change::PimdirWriteOp,
    client::PimdirSourceStore,
    placement::PimdirLinkId,
    reference::{PimdirEndpoint, PimdirReferenceOrigin, PimdirReferenceRole},
    summary::file::PimdirFileSummary,
};

use invariants_common::*;

const AUG: &str = "Sat, 1 Aug 2026 10:00:00 +0000";
const CARD: &str = "text/vcard";
const CAL: &str = "text/calendar";
const FILE: &str = "application/octet-stream";

fn end(kind: &str, link: &str) -> PimdirEndpoint {
    PimdirEndpoint {
        kind: kind.into(),
        link_id: PimdirLinkId(link.into()),
    }
}

fn card(uid: &str) -> Vec<u8> {
    format!("BEGIN:VCARD\r\nVERSION:4.0\r\nUID:{uid}\r\nFN:{uid}\r\nEND:VCARD\r\n").into_bytes()
}

fn event(uid: &str) -> Vec<u8> {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//x//x//EN\r\nBEGIN:VEVENT\r\n\
         UID:{uid}\r\nDTSTAMP:20260801T100000Z\r\nDTSTART:20260801T100000Z\r\n\
         SUMMARY:{uid}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    )
    .into_bytes()
}

/// A store with Cards and Cal kept on the device, each holding `same`,
/// Cards `alice` and Cal `ev`.
fn kinds(dir: &std::path::Path) -> (PimdirSourceStore, PimdirLinkId, PimdirLinkId) {
    let mut store = mail_store(dir, "imap");
    store.ensure_collection("Cards", CARD).unwrap();
    store.ensure_collection("Cal", CAL).unwrap();
    let (same_card, _) = store.put_item("Cards", &card("same")).unwrap();
    store.put_item("Cards", &card("alice")).unwrap();
    let (same_event, _) = store.put_item("Cal", &event("same")).unwrap();
    store.put_item("Cal", &event("ev")).unwrap();
    (store, same_card, same_event)
}

fn froms(store: &PimdirSourceStore, to: &PimdirEndpoint) -> Vec<String> {
    store
        .references_to(to)
        .unwrap()
        .into_iter()
        .map(|reference| format!("{}:{}", reference.from.kind, reference.from.link_id.0))
        .collect()
}

/// L1313: the last row of a kind going drops the references to that kind,
/// another kind's row under the same key holding nothing.
#[test]
fn l1313_a_to_end_goes_by_its_kind() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, same_card, same_event) = kinds(dir.path());
    let alice = PimdirLinkId::from_hint("alice");
    assert_eq!(same_card, same_event, "one key under two kinds");
    let related = PimdirReferenceRole::Related;
    let user = PimdirReferenceOrigin::User;
    store
        .add_reference(
            &end(CARD, alice.as_str()),
            &end(CAL, same_event.as_str()),
            &related,
            user,
        )
        .unwrap()
        .unwrap();
    store
        .add_reference(
            &end(CARD, alice.as_str()),
            &end(CARD, same_card.as_str()),
            &related,
            user,
        )
        .unwrap()
        .unwrap();

    assert!(store.delete_unbound("Cards", &same_card).unwrap());

    assert_eq!(
        froms(&store, &end(CAL, same_event.as_str())),
        [format!("{CARD}:{}", alice.0)],
        "the event under the key keeps its reference"
    );
    assert!(
        froms(&store, &end(CARD, same_card.as_str())).is_empty(),
        "the card's went with its last row"
    );
}

/// L1317: the last row of a from end's kind going drops its references,
/// another kind's row under the key holding nothing.
#[test]
fn l1317_a_from_end_goes_by_its_kind() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, same_card, same_event) = kinds(dir.path());
    let ev = PimdirLinkId::from_hint("ev");
    store
        .add_reference(
            &end(CARD, same_card.as_str()),
            &end(CAL, ev.as_str()),
            &PimdirReferenceRole::Related,
            PimdirReferenceOrigin::User,
        )
        .unwrap()
        .unwrap();
    assert_eq!(froms(&store, &end(CAL, ev.as_str())).len(), 1);

    assert!(store.delete_unbound("Cards", &same_card).unwrap());

    assert!(
        froms(&store, &end(CAL, ev.as_str())).is_empty(),
        "the event under {} holds no card's reference",
        same_event.0
    );
}

/// L1541: a stand-in a reference still names from its own end stays; once
/// none names it, it is collected.
#[test]
fn l1541_a_stand_in_named_from_its_own_end_stays() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    let m = message("m@x", "M", AUG, Some("alice@x"));
    store
        .write(vec![
            body(&store, &m),
            PimdirWriteOp::UpsertPlacement(held(&store, "INBOX", "1", &m)),
        ])
        .unwrap();
    store.ensure_collection("Attachments", FILE).unwrap();
    let part = PimdirLinkId("part:m@x#3".into());
    let local = PimdirLinkId("local".into());
    let summary = PimdirFileSummary {
        name: "photo.jpg".into(),
        media_type: Some("image/jpeg".into()),
        size: Some(3),
        part: Some("3".into()),
    };
    store.put_file("Attachments", &part, &summary).unwrap();
    store.put_file("Attachments", &local, &summary).unwrap();
    let held_stand_in = |store: &PimdirSourceStore| {
        store
            .seq_for_link("Attachments", part.as_str())
            .unwrap()
            .is_some()
    };
    let attachment = PimdirReferenceRole::Attachment;
    let related = PimdirReferenceRole::Related;
    store
        .add_reference(
            &end(MAIL, "m@x"),
            &end(FILE, part.as_str()),
            &attachment,
            PimdirReferenceOrigin::Auto,
        )
        .unwrap()
        .unwrap();
    store
        .add_reference(
            &end(FILE, part.as_str()),
            &end(FILE, local.as_str()),
            &related,
            PimdirReferenceOrigin::User,
        )
        .unwrap()
        .unwrap();

    store
        .remove_reference(&end(MAIL, "m@x"), &end(FILE, part.as_str()), &attachment)
        .unwrap()
        .unwrap();
    assert!(held_stand_in(&store), "still named from its own end");

    store
        .remove_reference(
            &end(FILE, part.as_str()),
            &end(FILE, local.as_str()),
            &related,
        )
        .unwrap()
        .unwrap();
    assert!(!held_stand_in(&store), "no reference names it: collected");
}

/// L1397: a write without the body keeps a known invitation.
#[test]
fn l1397_a_bodiless_write_keeps_a_known_invitation() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    let invite = format!(
        "Message-ID: <inv@x>\r\nFrom: alice@x\r\nTo: bob@x\r\nSubject: Invite\r\n\
         Date: {AUG}\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
         --b\r\nContent-Type: text/plain\r\n\r\nhi\r\n\
         --b\r\nContent-Type: text/calendar; method=REQUEST\r\n\r\n\
         BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:ev1\r\n\
         DTSTART:20260801T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n--b--\r\n"
    )
    .into_bytes();
    store
        .write(vec![
            body(&store, &invite),
            PimdirWriteOp::UpsertPlacement(held(&store, "INBOX", "1", &invite)),
        ])
        .unwrap();
    let mut bodiless = meta("INBOX", "1", &invite);
    bodiless.flags = io_pimdir::placement::PimdirFlags::from_iter(["\\Seen"]);
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(bodiless)])
        .unwrap();

    store.ensure_collection("Cal", CAL).unwrap();
    let (ev1, _) = store.put_item("Cal", &event("ev1")).unwrap();
    let invitations: Vec<String> = store
        .references_from(&end(MAIL, "inv@x"))
        .unwrap()
        .into_iter()
        .filter(|reference| reference.role == PimdirReferenceRole::Invitation)
        .map(|reference| reference.to.link_id.0)
        .collect();
    assert_eq!(
        invitations,
        [ev1.0],
        "the invitation kept through the bodiless write ties the event"
    );
}

/// L1512, L1564: a stand-in and its saved copy share one public id, and a
/// summary write restates the part.
#[test]
fn l1512_l1564_a_stand_in_and_its_copy_share_a_seq() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    store.ensure_collection("Attachments", FILE).unwrap();
    store.ensure_collection("Docs", FILE).unwrap();
    let part = PimdirLinkId("part:m@x#2".into());
    let summary = |part: &str| PimdirFileSummary {
        name: "report.pdf".into(),
        media_type: Some("application/pdf".into()),
        size: Some(3),
        part: Some(part.into()),
    };
    let stand_in = store.put_file("Attachments", &part, &summary("2")).unwrap();
    let saved = store.put_file("Docs", &part, &summary("2")).unwrap();
    assert_eq!(stand_in, saved, "L1512");

    assert_eq!(
        store.put_file("Docs", &part, &summary("1.2")).unwrap(),
        saved
    );
    let item = store.get_item("Docs", saved).unwrap().unwrap();
    match item.summary {
        Some(io_pimdir::summary::PimdirSummary::File(file)) => {
            assert_eq!(file.part.as_deref(), Some("1.2"), "L1564")
        }
        other => panic!("a file summary, not {other:?}"),
    }
}
