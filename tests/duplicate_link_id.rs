//! Persisting an identity a collection holds twice (spec §9, §10).
//!
//! The engine mints a key for the second copy before it writes, so the
//! store's whole obligation on this axis is to hold both items and to
//! refuse a write that would repoint a binding instead. Both halves are
//! the store's contract rather than the engine's: a consumer staging its
//! own writes reaches the same rows, and a rebuilt handle space resolves
//! two placements to one key without minting anything.

use io_pimdir::{PimdirError, PimdirReader, PimdirSourceStore, PimdirStore};
use io_replica::{
    change::{ReplicaDropReason, ReplicaWriteOp},
    client::ReplicaStorage,
    collection::ReplicaCollectionId,
    object::{ReplicaHash, ReplicaObject},
    placement::{
        ReplicaBase, ReplicaFlags, ReplicaHandle, ReplicaLevel, ReplicaLinkId, ReplicaMeta,
        ReplicaPlacement, ReplicaStatus,
    },
    storage::ReplicaLoadScope,
};
use tempfile::tempdir;

fn inbox() -> ReplicaCollectionId {
    ReplicaCollectionId("INBOX".into())
}

fn placement(handle: &str, link: &str) -> ReplicaPlacement {
    ReplicaPlacement {
        sort_key: Default::default(),
        collection: inbox(),
        handle: ReplicaHandle(handle.into()),
        link_id: Some(ReplicaLinkId(link.into())),
        object: None,
        level: ReplicaLevel::Meta,
        meta: Some(ReplicaMeta(r#"{"v":1}"#.into())),
        flags: ReplicaFlags::default(),
        status: ReplicaStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(ReplicaBase {
            flags: ReplicaFlags::default(),
            revision: None,
            object: None,
        }),
        origin: None,
    }
}

/// The same placement hydrated on `hash`, so two copies can be checked to
/// share one object or to hold one each.
fn hydrated(handle: &str, link: &str, hash: &str) -> ReplicaPlacement {
    let mut placement = placement(handle, link);
    placement.level = ReplicaLevel::Full;
    placement.object = Some(ReplicaHash(hash.into()));
    placement.base = Some(ReplicaBase {
        flags: ReplicaFlags::default(),
        revision: None,
        object: Some(ReplicaHash(hash.into())),
    });
    placement
}

fn store_object(hash: &str, body: &[u8]) -> ReplicaWriteOp {
    ReplicaWriteOp::StoreObject {
        object: ReplicaObject {
            hash: ReplicaHash(hash.into()),
            size: body.len(),
        },
        body: Some(body.to_vec()),
    }
}

fn projected(store: &PimdirSourceStore) -> Vec<ReplicaPlacement> {
    let mut placements = store
        .load(&inbox(), &ReplicaLoadScope::All)
        .unwrap()
        .placements;
    placements.sort_by(|a, b| a.link_id.cmp(&b.link_id));
    placements
}

fn opened(dir: &std::path::Path) -> PimdirSourceStore {
    let store = PimdirStore::open(dir).unwrap().for_source("remote");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    store
}

/// The floor: a binding pins one handle, and a write resolving it to
/// another is refused rather than applied and rather than recorded.
#[test]
fn a_colliding_write_is_refused_and_stores_nothing() {
    let dir = tempdir().unwrap();
    let mut store = opened(dir.path());

    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(placement(
            "u1", "msg-a",
        ))])
        .unwrap();

    let refused = store
        .write(vec![ReplicaWriteOp::UpsertPlacement(placement(
            "u2", "msg-a",
        ))])
        .unwrap_err();

    match &refused {
        PimdirError::Rebind {
            collection,
            link_id,
            source,
            bound,
            incoming,
        } => {
            assert_eq!((collection.as_str(), link_id.as_str()), ("INBOX", "msg-a"));
            assert_eq!(source, "remote");
            assert_eq!((bound.as_str(), incoming.as_str()), ("u1", "u2"));
        }
        other => panic!("a colliding write must be refused by type: {other:?}"),
    }
    assert!(
        refused.to_string().contains("u2"),
        "the message names the handle the write carried: {refused}",
    );

    let placements = projected(&store);
    assert_eq!(placements.len(), 1, "and nothing of it is stored");
    assert_eq!(placements[0].handle, ReplicaHandle("u1".into()));
}

/// A minted key is an ordinary key: the two copies are two items, with
/// their own `seq`, their own binding and their own body.
#[test]
fn two_resources_under_one_hint_are_two_items() {
    let dir = tempdir().unwrap();
    let mut store = opened(dir.path());

    store
        .write(vec![
            store_object("cafebabe", b"first"),
            store_object("deadbeef", b"second"),
            ReplicaWriteOp::UpsertPlacement(hydrated("u1", "msg-a", "cafebabe")),
            ReplicaWriteOp::UpsertPlacement(hydrated("u2", "dup:msg-a#u2", "deadbeef")),
        ])
        .unwrap();

    let placements = projected(&store);
    assert_eq!(placements.len(), 2);
    assert_eq!(
        placements[0].link_id,
        Some(ReplicaLinkId("dup:msg-a#u2".into()))
    );
    assert_eq!(placements[0].handle, ReplicaHandle("u2".into()));
    assert_eq!(placements[0].object, Some(ReplicaHash("deadbeef".into())));
    assert_eq!(placements[1].link_id, Some(ReplicaLinkId("msg-a".into())));
    assert_eq!(placements[1].object, Some(ReplicaHash("cafebabe".into())));

    let read = PimdirReader::open(dir.path()).unwrap();
    let bare = read.seq_for_link("INBOX", "msg-a").unwrap().unwrap();
    let minted = read.seq_for_link("INBOX", "dup:msg-a#u2").unwrap().unwrap();
    assert_ne!(
        bare, minted,
        "a key nothing else holds draws a public id of its own (spec §9.1)",
    );
    assert_eq!(
        read.item_bindings("INBOX", "dup:msg-a#u2").unwrap().len(),
        1,
        "and a binding of its own",
    );
}

/// The dedup axis is the body, so two copies that agree byte for byte
/// share one object, refcounted twice, while staying two items.
#[test]
fn a_byte_identical_pair_shares_one_object() {
    let dir = tempdir().unwrap();
    let mut store = opened(dir.path());

    store
        .write(vec![
            store_object("cafebabe", b"same"),
            ReplicaWriteOp::UpsertPlacement(hydrated("u1", "msg-a", "cafebabe")),
            ReplicaWriteOp::UpsertPlacement(hydrated("u2", "dup:msg-a#u2", "cafebabe")),
        ])
        .unwrap();

    let read = PimdirReader::open(dir.path()).unwrap();
    assert_eq!(read.object_stats().unwrap().count, 1);
    assert_eq!(
        read.object_placements("cafebabe").unwrap().len(),
        2,
        "one body, two placements: the report the store owes (spec §14.1)",
    );
    // the two items and the two bases the same write agreed on
    assert!(read.refcount_drift().unwrap().is_empty());
}

/// A minted key is opaque, so nothing along the way rewrites it: it goes
/// out of a page and back into a read exactly as it went in, retirement
/// and revival included.
#[test]
fn a_minted_key_round_trips_through_retention_and_revival() {
    let dir = tempdir().unwrap();
    let mut store = opened(dir.path());

    store
        .write(vec![
            store_object("cafebabe", b"body"),
            ReplicaWriteOp::UpsertPlacement(hydrated("u2", "dup:msg-a#u2", "cafebabe")),
        ])
        .unwrap();

    let paged = store.list_items("INBOX", None, 10).unwrap();
    assert_eq!(paged.len(), 1);
    assert_eq!(paged[0].link_id, ReplicaLinkId("dup:msg-a#u2".into()));

    // the source drops it: the row is retained rather than deleted, under
    // the same key
    store
        .write(vec![ReplicaWriteOp::DropPlacement {
            collection: inbox(),
            handle: ReplicaHandle("u2".into()),
            reason: ReplicaDropReason::Deleted,
        }])
        .unwrap();
    let retained = store.list_retained(&inbox(), None, 10).unwrap();
    assert_eq!(retained.len(), 1);
    assert_eq!(retained[0].link_id, ReplicaLinkId("dup:msg-a#u2".into()));
    let seq = retained[0].seq;

    // and the source hands the same resource back: the row revives on the
    // key it kept, with the public id it kept
    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(hydrated(
            "u2",
            "dup:msg-a#u2",
            "cafebabe",
        ))])
        .unwrap();
    let revived = store.list_items("INBOX", None, 10).unwrap();
    assert_eq!(revived.len(), 1);
    assert_eq!(revived[0].link_id, ReplicaLinkId("dup:msg-a#u2".into()));
    assert_eq!(revived[0].seq, seq, "a revived item keeps its public id");
}

/// A rebuilt handle space is a repoint the floor MUST let through.
///
/// A rekey drops the whole old spine and upserts every item under its new
/// handle, in one batch (spec §12). Read without knowing that, the two
/// halves are indistinguishable from one source reporting an identity
/// under a second handle, and a UIDVALIDITY bump would refuse the write
/// for every item of the collection, under handles the server has just
/// voided.
#[test]
fn a_rekey_carries_the_binding_over_instead_of_refusing_it() {
    let dir = tempdir().unwrap();
    let mut store = opened(dir.path());

    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(placement(
            "u1", "msg-a",
        ))])
        .unwrap();

    store
        .write_rekeyed(
            "INBOX",
            vec![
                ReplicaWriteOp::DropPlacement {
                    collection: inbox(),
                    handle: ReplicaHandle("u1".into()),
                    reason: ReplicaDropReason::Superseded,
                },
                ReplicaWriteOp::UpsertPlacement(placement("101", "msg-a")),
            ],
        )
        .unwrap();

    let placements = projected(&store);
    assert_eq!(placements.len(), 1);
    assert_eq!(
        placements[0].handle,
        ReplicaHandle("101".into()),
        "the binding follows the rebuilt spine",
    );
    assert_eq!(placements[0].status, ReplicaStatus::Clean);
}

/// The licence is per handle, not per batch.
///
/// A rekey batch also carrying a genuine second copy of one identity must
/// still be refused for that one: superseding `u1` says nothing about
/// `u9`, and reading the reason as a blanket permission would put the
/// loss back inside the one operation that legitimately repoints.
#[test]
fn a_superseded_handle_licenses_only_its_own_rebind() {
    let dir = tempdir().unwrap();
    let mut store = opened(dir.path());

    store
        .write(vec![
            ReplicaWriteOp::UpsertPlacement(placement("u1", "msg-a")),
            ReplicaWriteOp::UpsertPlacement(placement("u2", "msg-b")),
        ])
        .unwrap();

    let refused = store
        .write_rekeyed(
            "INBOX",
            vec![
                // msg-a is superseded and renumbered, so it carries over
                ReplicaWriteOp::DropPlacement {
                    collection: inbox(),
                    handle: ReplicaHandle("u1".into()),
                    reason: ReplicaDropReason::Superseded,
                },
                ReplicaWriteOp::UpsertPlacement(placement("101", "msg-a")),
                // msg-b is not: the source holds it under a second handle
                ReplicaWriteOp::UpsertPlacement(placement("u9", "msg-b")),
            ],
        )
        .unwrap_err();

    assert!(
        matches!(&refused, PimdirError::Rebind { link_id, bound, incoming, .. }
            if link_id == "msg-b" && bound == "u2" && incoming == "u9"),
        "the refusal names the copy the licence does not cover: {refused}",
    );

    let placements = projected(&store);
    assert_eq!(
        placements[0].handle,
        ReplicaHandle("u1".into()),
        "and the refused batch leaves the licensed half unwritten too, \
         a write being one transaction",
    );
    assert_eq!(placements[1].handle, ReplicaHandle("u2".into()));
}

/// A rebuild mints nothing, so a collection that genuinely holds an
/// identity twice hands the store two placements resolving to one key in
/// one batch (io-replica, `rekey`). The hub is keyed by link id, so
/// folding both would keep whichever the batch names last and drop the
/// other with no statement failing.
#[test]
fn a_rebuilt_handle_space_resolving_two_placements_to_one_key_is_refused() {
    let dir = tempdir().unwrap();
    let mut store = opened(dir.path());

    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(placement(
            "u1", "msg-a",
        ))])
        .unwrap();

    let refused = store
        .write_rekeyed(
            "INBOX",
            vec![
                ReplicaWriteOp::DropPlacement {
                    collection: inbox(),
                    handle: ReplicaHandle("u1".into()),
                    reason: ReplicaDropReason::Superseded,
                },
                // both copies report the bare hint again, the rebuild
                // having re-resolved every identity from the new spine
                ReplicaWriteOp::UpsertPlacement(placement("101", "msg-a")),
                ReplicaWriteOp::UpsertPlacement(placement("102", "msg-a")),
            ],
        )
        .unwrap_err();

    assert!(
        matches!(&refused, PimdirError::Rebind { link_id, bound, incoming, .. }
            if link_id == "msg-a" && bound == "101" && incoming == "102"),
        "the refusal names both handles the batch claimed the key under: {refused}",
    );

    let placements = projected(&store);
    assert_eq!(placements.len(), 1);
    assert_eq!(
        placements[0].handle,
        ReplicaHandle("u1".into()),
        "the store holds what it held: no overwrite, no half-written spine",
    );
}
