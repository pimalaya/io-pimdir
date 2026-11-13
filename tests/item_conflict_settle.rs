//! A person's resolution, an `Edit` naming the diverging body in `resolves`,
//! settles the item's conflict whatever body it keeps (SYNC §7):
//! `items.conflicted` cleared and its `conflict_object` released, nothing it
//! discards kept. The item takes the edit's body, the editing source agrees
//! on it, and every other source then owes it where its base differs.
//!
//! Two sources diverge on one card: the phone pulled its server's edit P
//! into the item, then the server's own edit S met it under `manual`, so
//! the item holds P with S as its `conflict_object`, and the server's
//! binding still agrees on the ancestor B. A person then keeps P, takes S
//! or writes a merge X, through either source: six cells, each settling.
//! The overruling side is vectors 102 and 103.

#![cfg(feature = "client")]

use std::path::Path;

use io_pimdir::client::{
    PimdirRunError, PimdirSourceStore, PimdirStore, producer::PimdirProducer, reader::PimdirReader,
};
use io_pimdir::{
    change::PimdirWriteOp,
    codec::PimdirAction,
    collection::PimdirCollectionId,
    hash,
    hub::PimdirSourceId,
    load::PimdirLoadSelection,
    mutate::{PimdirMutateError, PimdirMutation},
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

    // NOTE: the diverging body the person was shown (SYNC §7).
    let shown: Vec<PimdirHash> = editor
        .list_item_conflicts(None)
        .unwrap()
        .into_iter()
        .filter_map(|conflict| conflict.conflict_object)
        .collect();
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
                resolves: Some(shown),
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

/// The card's public id.
fn seq(store: &PimdirSourceStore) -> i64 {
    store
        .seq_for_link(CONTACTS, LINK)
        .unwrap()
        .expect("the card has a public id")
}

/// Queues an `update` of the card to `bytes` over the phone's body, a
/// resolution of `resolves` when it states one, answering the row's id.
fn queue_update(dir: &Path, seq: i64, bytes: &[u8], resolves: Option<&[&[u8]]>) -> i64 {
    let mut producer = PimdirProducer::open(dir, "test").unwrap();
    let if_match = Some(hash::hash(PHONE));
    let resolves = resolves.map(|bodies| bodies.iter().map(|body| hash::hash(body)).collect());
    producer
        .enqueue_with_body(CONTACTS, bytes, |object| PimdirAction::Update {
            seq,
            object,
            if_match,
            resolves,
        })
        .unwrap()
        .handle
        .id
}

/// The card's body as a reader overlaying the queue reads it (§15.4).
fn overlaid(dir: &Path, seq: i64) -> Option<PimdirHash> {
    let reader = PimdirReader::open(dir).unwrap().with_pending();
    reader.get_item(CONTACTS, seq).unwrap().unwrap().object
}

/// A queued `update` meeting the item's conflict parks naming it unless it
/// resolves it, and one resolving a conflict that no longer stands parks
/// too (STORAGE §15.3, SYNC §7): an ordinary edit queued before the
/// conflict arose never discards the diverging body. The overlay folds
/// only what the drain applies.
#[test]
fn a_queued_update_settles_only_the_conflict_it_resolves() {
    let dir = tempfile::tempdir().unwrap();
    let (mut phone, _server) = diverged(dir.path());
    let seq = seq(&phone);
    let server_body = phone.hash(SERVER);

    // NOTE: an ordinary edit, made over the body the item keeps.
    queue_update(dir.path(), seq, MERGED, None);
    assert_eq!(
        overlaid(dir.path(), seq),
        Some(phone.hash(PHONE)),
        "not folded"
    );
    let report = phone.drain().unwrap();
    assert_eq!((report.applied, report.parked), (0, 1));
    let parked = phone.parked_actions().unwrap();
    assert!(
        parked[0].error.contains(&server_body.0),
        "{}",
        parked[0].error
    );
    let conflicts = phone.list_item_conflicts(None).unwrap();
    assert_eq!(conflicts.len(), 1, "the conflict stands");
    assert_eq!(conflicts[0].conflict_object, Some(server_body.clone()));
    assert_eq!(conflicts[0].object, Some(phone.hash(PHONE)));

    // a resolution naming another body parks the same
    queue_update(dir.path(), seq, MERGED, Some(&[ANCESTOR]));
    assert_eq!(
        overlaid(dir.path(), seq),
        Some(phone.hash(PHONE)),
        "not folded"
    );
    let report = phone.drain().unwrap();
    assert_eq!((report.applied, report.parked), (0, 1));

    // the person's resolution of the conflict shown settles it
    queue_update(dir.path(), seq, MERGED, Some(&[SERVER]));
    assert_eq!(
        overlaid(dir.path(), seq),
        Some(phone.hash(MERGED)),
        "folded"
    );
    let report = phone.drain().unwrap();
    assert_eq!((report.applied, report.parked), (1, 0));
    assert!(phone.list_item_conflicts(None).unwrap().is_empty());
    let item = phone.get_item(CONTACTS, seq).unwrap().unwrap();
    assert_eq!(item.object, Some(phone.hash(MERGED)));

    // and one resolving it again meets no conflict: parked
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    producer
        .enqueue_with_body(CONTACTS, PHONE, |object| PimdirAction::Update {
            seq,
            object,
            if_match: Some(hash::hash(MERGED)),
            resolves: Some(vec![server_body.clone()]),
        })
        .unwrap();
    drop(producer);
    assert_eq!(
        overlaid(dir.path(), seq),
        Some(phone.hash(MERGED)),
        "not folded"
    );
    let report = phone.drain().unwrap();
    assert_eq!((report.applied, report.parked), (0, 1));
    let item = phone.get_item(CONTACTS, seq).unwrap().unwrap();
    assert_eq!(item.object, Some(phone.hash(MERGED)));
}

/// A queued resolution runs through the conflicted binding whose body it
/// names (STORAGE §15.2): the owner draining as the phone, whose binding
/// is in no conflict, settles the server's, the base adopting the
/// revision and body it recorded.
#[test]
fn a_queued_resolution_runs_through_the_binding_it_names() {
    let dir = tempfile::tempdir().unwrap();
    let mut phone = store(dir.path(), "phone");
    let mut server = store(dir.path(), "server");
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
    // NOTE: the server's own edit S at e2 met the store's, recorded on its
    // binding alone.
    let PimdirWriteOp::UpsertPlacement(mut diverged) = card(&server, "a.vcf", ANCESTOR, "e1")
    else {
        unreachable!()
    };
    diverged.status = PimdirStatus::Conflict;
    diverged.conflict_revision = Some("e2".into());
    diverged.conflict_object = Some(server.hash(SERVER));
    server
        .write(vec![
            body(&server, SERVER),
            PimdirWriteOp::UpsertPlacement(diverged),
        ])
        .unwrap();
    assert!(phone.list_item_conflicts(None).unwrap().is_empty());
    assert_eq!(phone.list_conflicts(None).unwrap().len(), 1);
    drop(server);

    let seq = seq(&phone);
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    producer
        .enqueue_with_body(CONTACTS, MERGED, |object| PimdirAction::Update {
            seq,
            object,
            if_match: Some(hash::hash(ANCESTOR)),
            resolves: Some(vec![hash::hash(SERVER)]),
        })
        .unwrap();
    drop(producer);
    assert_eq!(
        overlaid(dir.path(), seq),
        Some(phone.hash(MERGED)),
        "folded"
    );

    let report = phone.drain().unwrap();
    assert_eq!(
        (report.applied, report.parked),
        (1, 0),
        "{:?}",
        phone.parked_actions()
    );
    assert!(phone.list_conflicts(None).unwrap().is_empty(), "settled");
    let hub = phone.load_hub(CONTACTS).unwrap();
    let item = &hub.items[&PimdirLinkId(LINK.into())];
    assert_eq!(item.object, Some(phone.hash(MERGED)));
    let base = item.sources[&PimdirSourceId("server".into())]
        .base
        .clone()
        .expect("a base");
    assert_eq!(base.revision.as_deref(), Some("e2"));
    assert_eq!(base.object, Some(phone.hash(SERVER)));
}

/// Queues a `remove` of the card stating `resolves`, answering nothing.
fn queue_remove(dir: &Path, seq: i64, if_match: Option<&[u8]>, resolves: Option<&[&[u8]]>) {
    let mut producer = PimdirProducer::open(dir, "test").unwrap();
    let action = PimdirAction::Remove {
        seq,
        if_match: if_match.map(hash::hash),
        resolves: resolves.map(|bodies| bodies.iter().map(|body| hash::hash(body)).collect()),
    };
    producer.enqueue(CONTACTS, &action, None).unwrap();
}

/// Whether a reader overlaying the queue still lists the card (§15.4).
fn listed(dir: &Path, seq: i64) -> bool {
    let reader = PimdirReader::open(dir).unwrap().with_pending();
    reader.get_item(CONTACTS, seq).unwrap().is_some()
}

/// A queued `remove` meeting the item's conflict parks naming it unless it
/// is a resolution naming it (STORAGE §15.2, §15.3; SYNC §7; vectors/sync
/// 143, 146): a delete queued before the conflict arose never deletes the
/// diverging body unseen. One naming it tombstones the item, its conflict
/// cleared and the diverging body released, the item keeping its own. The
/// overlay takes the item out only where the drain applies the row.
///
/// Before draft-10's second batch every remove here applied, and the
/// tombstone's upsert cleared the conflict whoever staged it.
#[test]
fn a_queued_remove_settles_only_the_conflict_it_resolves() {
    let dir = tempfile::tempdir().unwrap();
    let (mut phone, _server) = diverged(dir.path());
    let seq = seq(&phone);
    let server_body = phone.hash(SERVER);

    // NOTE: an ordinary remove, of the body the item keeps.
    queue_remove(dir.path(), seq, Some(PHONE), None);
    assert!(listed(dir.path(), seq), "not folded");
    let report = phone.drain().unwrap();
    assert_eq!((report.applied, report.parked), (0, 1));
    let parked = phone.parked_actions().unwrap();
    assert!(
        parked[0].error.contains(&server_body.0),
        "{}",
        parked[0].error
    );
    let conflicts = phone.list_item_conflicts(None).unwrap();
    assert_eq!(conflicts.len(), 1, "the conflict stands");
    assert_eq!(conflicts[0].conflict_object, Some(server_body.clone()));
    assert_eq!(refcount(dir.path(), &server_body), 2, "both bodies kept");

    // a resolution naming another body parks the same
    queue_remove(dir.path(), seq, None, Some(&[ANCESTOR]));
    assert!(listed(dir.path(), seq), "not folded");
    let report = phone.drain().unwrap();
    assert_eq!((report.applied, report.parked), (0, 1));

    // a remove stating a body the item does not hold parks before any
    // conflict is read
    queue_remove(dir.path(), seq, Some(MERGED), Some(&[SERVER]));
    assert!(listed(dir.path(), seq), "not folded");
    let report = phone.drain().unwrap();
    assert_eq!((report.applied, report.parked), (0, 1));
    let parked = phone.parked_actions().unwrap();
    let replaced = parked.last().unwrap();
    assert!(
        replaced.error.contains("body replaced"),
        "{}",
        replaced.error
    );

    // the person's resolution by deleting settles it
    queue_remove(dir.path(), seq, Some(PHONE), Some(&[SERVER]));
    assert!(!listed(dir.path(), seq), "folded: the item is taken out");
    let report = phone.drain().unwrap();
    assert_eq!(
        (report.applied, report.parked),
        (1, 0),
        "{:?}",
        phone.parked_actions()
    );
    let hub = phone.load_hub(CONTACTS).unwrap();
    let item = &hub.items[&PimdirLinkId(LINK.into())];
    assert!(item.deleted, "tombstoned");
    assert!(!item.conflicted, "the item's conflict cleared");
    assert_eq!(item.conflict_object, None, "its body released");
    assert_eq!(item.object, Some(phone.hash(PHONE)), "its own body kept");
    assert_eq!(
        refcount(dir.path(), &server_body),
        1,
        "the server's base alone holds the diverging body"
    );
    assert!(phone.refcount_drift().unwrap().is_empty());
    assert_eq!(status(&phone), PimdirStatus::Tombstone);
}

/// A `Move` meeting the item's conflict is refused, staging nothing in
/// either collection, and a queued `move` parks naming the conflict (SYNC
/// §7; STORAGE §15.2, §15.3; vectors/sync 155, 156). It states no
/// resolution: its tombstone would leave the conflict standing, the
/// diverging body pinned under a placement no listing shows. A reader's
/// overlay folds nothing for it, in the collection it leaves and in the
/// one it names (§15.4). A `copy` meets no conflict, and the move applies
/// once a person's resolution settled it.
#[test]
fn a_move_over_a_conflicted_item_is_refused_and_parks() {
    let dir = tempfile::tempdir().unwrap();
    let (mut phone, _server) = diverged(dir.path());
    phone.ensure_collection("archive", "text/vcard").unwrap();
    phone.ensure_collection("kept", "text/vcard").unwrap();
    let archive = PimdirCollectionId("archive".into());
    let seq = seq(&phone);
    let server_body = phone.hash(SERVER);
    let standing = |phone: &PimdirSourceStore| {
        let hub = phone.load_hub(CONTACTS).unwrap();
        let item = &hub.items[&PimdirLinkId(LINK.into())];
        assert!(!item.deleted, "the card stays where it stood");
        assert!(item.conflicted, "the conflict stands");
        assert_eq!(item.conflict_object, Some(server_body.clone()));
        assert_eq!(refcount(dir.path(), &server_body), 2, "both bodies kept");
        assert!(phone.load_hub("archive").unwrap().items.is_empty());
    };

    let refused = phone.mutate(
        CONTACTS,
        PimdirMutation::Move {
            handle: PimdirHandle("1".into()),
            target: archive.clone(),
        },
    );
    let Err(PimdirRunError::Coroutine(refused)) = refused else {
        panic!("a move over a conflict staged: {refused:?}");
    };
    assert_eq!(refused.code(), "conflict-unresolved");
    let PimdirMutateError::Unresolved { met, resolves, .. } = refused else {
        panic!("expected Unresolved, got {refused:?}");
    };
    assert_eq!(met, [Some(server_body.clone())]);
    assert_eq!(resolves, None, "a move states none");
    standing(&phone);

    let reader = PimdirReader::open(dir.path()).unwrap().with_pending();
    let queue = |action: &PimdirAction| {
        let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
        producer.enqueue(CONTACTS, action, None).unwrap();
    };
    queue(&PimdirAction::Move {
        seq,
        to: archive.clone(),
    });
    assert!(
        listed(dir.path(), seq),
        "not folded: the card is not taken out"
    );
    assert!(reader.get_item("archive", seq).unwrap().is_none(), "nor in");
    assert_eq!(reader.count_items("archive").unwrap(), 0);
    assert!(reader.list_items("archive", None, 10).unwrap().is_empty());
    let report = phone.drain().unwrap();
    assert_eq!((report.applied, report.parked), (0, 1));
    let parked = phone.parked_actions().unwrap();
    assert_eq!(parked[0].action, "move");
    assert!(
        parked[0].error.contains(&server_body.0),
        "{}",
        parked[0].error
    );
    standing(&phone);

    // a copy tombstones nothing and meets no conflict
    queue(&PimdirAction::Copy {
        seq,
        to: PimdirCollectionId("kept".into()),
    });
    assert!(reader.get_item("kept", seq).unwrap().is_some(), "folded");
    let report = phone.drain().unwrap();
    assert_eq!((report.applied, report.parked), (1, 0));
    assert_eq!(phone.load_hub("kept").unwrap().items.len(), 1);
    standing(&phone);

    // settled by a person, then moved: both rows fold and apply
    queue_update(dir.path(), seq, MERGED, Some(&[SERVER]));
    queue(&PimdirAction::Move {
        seq,
        to: archive.clone(),
    });
    assert!(!listed(dir.path(), seq), "folded: taken out");
    let arrived = reader.get_item("archive", seq).unwrap();
    assert!(arrived.is_some(), "folded: brought in");
    assert_eq!(reader.count_items("archive").unwrap(), 1);
    let report = phone.drain().unwrap();
    assert_eq!(
        (report.applied, report.parked),
        (2, 0),
        "{:?}",
        phone.parked_actions()
    );
    let hub = phone.load_hub(CONTACTS).unwrap();
    let item = &hub.items[&PimdirLinkId(LINK.into())];
    assert!(item.deleted && !item.conflicted, "moved once settled");
    assert_eq!(phone.load_hub("archive").unwrap().items.len(), 1);
    assert!(phone.refcount_drift().unwrap().is_empty());
}

/// An `add` superseding a conflicted item tombstones it as a `remove` of
/// it would (STORAGE §15.3): stating no resolution, the remove is refused
/// and the whole action parks, its add with it, the conflict and both
/// bodies kept.
#[test]
fn an_add_superseding_a_conflicted_item_parks_whole() {
    let dir = tempfile::tempdir().unwrap();
    let (mut phone, _server) = diverged(dir.path());
    phone.ensure_collection("archive", "text/vcard").unwrap();
    let seq = seq(&phone);

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    producer
        .enqueue_with_body("archive", MERGED, |object| PimdirAction::Add {
            link_id: None,
            flags: PimdirFlags::default(),
            object: Some(object),
            supersedes: Some(io_pimdir::codec::PimdirSupersedes {
                collection: PimdirCollectionId(CONTACTS.into()),
                seq,
            }),
        })
        .unwrap();
    drop(producer);

    let report = phone.drain().unwrap();
    assert_eq!((report.applied, report.parked), (0, 1));
    assert!(
        phone.load_hub("archive").unwrap().items.is_empty(),
        "the add was not applied alone"
    );
    let hub = phone.load_hub(CONTACTS).unwrap();
    let item = &hub.items[&PimdirLinkId(LINK.into())];
    assert!(!item.deleted && item.conflicted, "left as it stood");
}

/// A run of actions is one transaction, a savepoint per action (STORAGE
/// §15.2): the superseding add above writes its item, then fails on the
/// remove it implies. Rolled back to its savepoint, what it wrote is
/// undone and its row restored and parked, while the adds queued around
/// it, applied in the same transaction, stand with their receipts.
#[test]
fn an_action_parking_after_it_wrote_is_undone_alone_in_its_run() {
    use io_pimdir::client::producer::PimdirActionStatus;

    let dir = tempfile::tempdir().unwrap();
    let (mut phone, _server) = diverged(dir.path());
    phone.ensure_collection("archive", "text/vcard").unwrap();
    let seq = seq(&phone);
    let card = |uid: &str| {
        format!("BEGIN:VCARD\r\nVERSION:4.0\r\nUID:urn:uuid:{uid}\r\nFN:{uid}\r\nEND:VCARD\r\n")
    };
    let add = |supersedes| {
        move |object| PimdirAction::Add {
            link_id: None,
            flags: PimdirFlags::default(),
            object: Some(object),
            supersedes,
        }
    };

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let before = producer
        .enqueue_with_body("archive", card("before").as_bytes(), add(None))
        .unwrap()
        .handle;
    let superseding = io_pimdir::codec::PimdirSupersedes {
        collection: PimdirCollectionId(CONTACTS.into()),
        seq,
    };
    let parking = producer
        .enqueue_with_body("archive", MERGED, add(Some(superseding)))
        .unwrap()
        .handle;
    let after = producer
        .enqueue_with_body("archive", card("after").as_bytes(), add(None))
        .unwrap()
        .handle;

    let report = phone.drain().unwrap();
    assert_eq!((report.applied, report.parked), (2, 1));

    let archived: Vec<String> = phone
        .load_hub("archive")
        .unwrap()
        .items
        .into_keys()
        .map(|link| link.0)
        .collect();
    assert_eq!(
        archived,
        vec!["urn:uuid:after", "urn:uuid:before"],
        "the add that parked left no item"
    );
    for applied in [&before, &after] {
        let status = producer.action_status(applied).unwrap();
        assert!(
            matches!(status, PimdirActionStatus::Applied { seq: Some(_), .. }),
            "{status:?}"
        );
    }
    let status = producer.action_status(&parking).unwrap();
    assert!(
        matches!(status, PimdirActionStatus::Parked { attempts: 1, .. }),
        "the claimed row restored, then parked: {status:?}"
    );
    let hub = phone.load_hub(CONTACTS).unwrap();
    let item = &hub.items[&PimdirLinkId(LINK.into())];
    assert!(!item.deleted && item.conflicted, "left as it stood");
}
