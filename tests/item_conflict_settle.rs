//! A person's `Edit` settles the item's conflict whatever body it keeps
//! (SYNC §7): `items.conflicted` cleared and its `conflict_object` released,
//! nothing it discards kept. The item takes the edit's body, the editing
//! source agrees on it, and every other source then owes it where its base
//! differs.
//!
//! Two sources diverge on one card: the phone pulled its server's edit P
//! into the item, then the server's own edit S met it under `manual`, so
//! the item holds P with S as its `conflict_object`, and the server's
//! binding still agrees on the ancestor B. A person then keeps P, takes S
//! or writes a merge X, through either source: six cells, each settling.
//! The overruling side is vectors 102 and 103.

#![cfg(feature = "client")]

use std::path::Path;

use io_pimdir::client::{PimdirSourceStore, PimdirStore};
use io_pimdir::{
    change::PimdirWriteOp,
    collection::PimdirCollectionId,
    hub::PimdirSourceId,
    load::PimdirLoadSelection,
    mutate::PimdirMutation,
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirSortKey, PimdirStatus,
    },
};
use rusqlite::Connection;

const CONTACTS: &str = "contacts";
const LINK: &str = "urn:uuid:c1";

const ANCESTOR: &[u8] = b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:urn:uuid:c1\r\nFN:Jo\r\nEND:VCARD\r\n";
const PHONE: &[u8] = b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:urn:uuid:c1\r\nFN:Joe\r\nEND:VCARD\r\n";
const SERVER: &[u8] =
    b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:urn:uuid:c1\r\nFN:Joseph\r\nEND:VCARD\r\n";
const MERGED: &[u8] =
    b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:urn:uuid:c1\r\nFN:Joe Joseph\r\nEND:VCARD\r\n";

fn store(dir: &Path, source: &str) -> PimdirSourceStore {
    let store = PimdirStore::open(dir).unwrap().for_source(source);
    store.ensure_collection(CONTACTS, "text/vcard").unwrap();
    store
}

fn body(store: &PimdirSourceStore, bytes: &[u8]) -> PimdirWriteOp {
    PimdirWriteOp::StoreObject {
        object: PimdirObject {
            hash: store.hash(bytes),
            size: bytes.len(),
        },
        body: Some(bytes.to_vec()),
    }
}

/// One source's placement of the card, in sync with its own server at
/// `revision` holding `bytes`.
fn card(store: &PimdirSourceStore, handle: &str, bytes: &[u8], revision: &str) -> PimdirWriteOp {
    PimdirWriteOp::UpsertPlacement(PimdirPlacement {
        collection: PimdirCollectionId(CONTACTS.into()),
        handle: PimdirHandle(handle.into()),
        link_id: Some(PimdirLinkId(LINK.into())),
        object: Some(store.hash(bytes)),
        level: PimdirLevel::Full,
        summary: None,
        sort_key: PimdirSortKey("k".into()),
        flags: PimdirFlags::default(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: Some(revision.into()),
            object: Some(store.hash(bytes)),
        }),
        origin: None,
    })
}

/// The phone and the server, diverged on the card: the item holds the
/// phone's P, its `conflict_object` the server's S, and the server's
/// binding agrees still on the ancestor B.
fn diverged(dir: &Path) -> (PimdirSourceStore, PimdirSourceStore) {
    let mut phone = store(dir, "phone");
    let mut server = store(dir, "server");

    phone
        .write(vec![
            body(&phone, ANCESTOR),
            card(&phone, "1", ANCESTOR, "v1"),
        ])
        .unwrap();
    server
        .write(vec![
            body(&server, ANCESTOR),
            card(&server, "a.vcf", ANCESTOR, "e1"),
        ])
        .unwrap();
    // NOTE: the phone's pull fast-forwards the item, then the server's
    // meets it, both having changed: a divergence under `manual`.
    phone
        .write(vec![body(&phone, PHONE), card(&phone, "1", PHONE, "v2")])
        .unwrap();
    let written = server
        .write(vec![
            body(&server, SERVER),
            card(&server, "a.vcf", SERVER, "e2"),
        ])
        .unwrap();
    assert_eq!(written.conflicted, [PimdirHandle("a.vcf".into())]);

    let hub = phone.load_hub(CONTACTS).unwrap();
    let item = &hub.items[&PimdirLinkId(LINK.into())];
    assert_eq!(item.object, Some(phone.hash(PHONE)));
    assert!(item.conflicted);
    assert_eq!(item.conflict_object, Some(phone.hash(SERVER)));
    let agreed = &item.sources[&PimdirSourceId("server".into())].shared_object;
    assert_eq!(agreed, &Some(phone.hash(ANCESTOR)));

    (phone, server)
}

fn refcount(dir: &Path, hash: &PimdirHash) -> i64 {
    Connection::open(dir.join("pimdir.db"))
        .unwrap()
        .query_row(
            "SELECT refcount FROM objects WHERE hash = ?1",
            [&hash.0],
            |r| r.get(0),
        )
        .unwrap()
}

/// The status a source's projection of the card reads.
fn status(store: &PimdirSourceStore) -> PimdirStatus {
    let loaded = store
        .load(
            &PimdirCollectionId(CONTACTS.into()),
            &PimdirLoadSelection::All,
        )
        .unwrap();
    assert_eq!(loaded.placements.len(), 1, "{:?}", loaded.placements);
    loaded.placements[0].status
}

/// A person's edit of the card through `through`, keeping `pick`; every
/// cell is asserted to settle as SYNC §7 says.
fn settle(through: &str, (kept, pick): (&str, &[u8])) {
    let dir = tempfile::tempdir().unwrap();
    let (mut phone, mut server) = diverged(dir.path());
    let (editor, handle) = match through {
        "phone" => (&mut phone, "1"),
        _ => (&mut server, "a.vcf"),
    };

    let picked = editor.hash(pick);
    let report = editor
        .mutate(
            CONTACTS,
            PimdirMutation::Edit {
                handle: PimdirHandle(handle.into()),
                object: PimdirObject {
                    hash: picked.clone(),
                    size: pick.len(),
                },
                body: pick.to_vec(),
                summary: None,
                sort_key: None,
                if_match: None,
                overrule: false,
            },
        )
        .unwrap();
    let cell = format!("{kept} through the {through}");

    assert!(report.events.is_empty(), "{cell}: a person keeps nothing");

    let hub = phone.load_hub(CONTACTS).unwrap();
    assert_eq!(hub.items.len(), 1, "{cell}: nothing retained");
    let item = &hub.items[&PimdirLinkId(LINK.into())];
    assert_eq!(item.object, Some(picked.clone()), "{cell}: the edit's body");
    assert!(!item.conflicted, "{cell}: the item's conflict cleared");
    assert_eq!(item.conflict_object, None, "{cell}: its body released");
    let agreed = &item.sources[&PimdirSourceId(through.into())].shared_object;
    assert_eq!(agreed, &Some(picked.clone()), "{cell}: the editor agrees");
    assert!(
        phone.list_item_conflicts(None).unwrap().is_empty(),
        "{cell}"
    );

    // NOTE: the server's base holds S whatever the pick, the item's
    // reference to it going with the conflict.
    let server_body = phone.hash(SERVER);
    let pins = 1 + 2 * i64::from(picked == server_body);
    assert_eq!(
        refcount(dir.path(), &server_body),
        pins,
        "{cell}: the diverging body released"
    );
    assert!(phone.refcount_drift().unwrap().is_empty(), "{cell}");

    // NOTE: each source owes the body where its base differs.
    for (source, base) in [(&phone, PHONE), (&server, SERVER)] {
        let owes = match phone.hash(base) == picked {
            true => PimdirStatus::Clean,
            false => PimdirStatus::Dirty,
        };
        assert_eq!(status(source), owes, "{cell}: {}", source.source());
    }
}

#[test]
fn keeping_the_items_body_through_the_agreeing_source_settles() {
    settle("phone", ("P", PHONE));
}

#[test]
fn taking_the_diverging_body_through_the_agreeing_source_settles() {
    settle("phone", ("S", SERVER));
}

#[test]
fn a_merge_through_the_agreeing_source_settles() {
    settle("phone", ("X", MERGED));
}

#[test]
fn keeping_the_items_body_through_the_source_it_left_settles() {
    settle("server", ("P", PHONE));
}

#[test]
fn taking_the_diverging_body_through_the_source_it_left_settles() {
    settle("server", ("S", SERVER));
}

#[test]
fn a_merge_through_the_source_it_left_settles() {
    settle("server", ("X", MERGED));
}
