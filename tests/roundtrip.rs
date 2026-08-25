//! Single-source round-trip (write, reopen, load, lookup, drop, GC) and
//! two-source propagation (a copy appears on the other side, a delete
//! lingers as a tombstone): the N=1 and N=2 cases of the same store.

use std::path::Path;

use io_pimdir::{PimdirError, PimdirSourceStore, PimdirStore, hash::PimdirHashAlgo};
use io_replica::{
    change::{ReplicaDropReason, ReplicaWriteOp},
    client::ReplicaStorage,
    collection::{ReplicaCheckpoint, ReplicaCollectionId},
    coroutine::{ReplicaArg, ReplicaCoroutine, ReplicaCoroutineState, ReplicaYield},
    mutate::{ReplicaMutate, ReplicaMutation},
    object::{ReplicaHash, ReplicaObject},
    placement::{
        ReplicaBase, ReplicaFlags, ReplicaHandle, ReplicaLevel, ReplicaLinkId, ReplicaMeta,
        ReplicaPlacement, ReplicaStatus,
    },
    storage::ReplicaLoadScope,
};

/// Drives a `ReplicaMutate` against the store to completion, the same
/// load/write pump a client runs, so a test exercises the real staged
/// mutation path end to end.
fn run_mutation(store: &mut PimdirSourceStore, collection: &str, mutation: ReplicaMutation) {
    let mut coroutine = ReplicaMutate::new(collection.to_string(), mutation);
    let mut arg: Option<ReplicaArg> = None;
    loop {
        match coroutine.resume(arg.take()) {
            ReplicaCoroutineState::Yielded(ReplicaYield::WantsLoad { collection, scope }) => {
                arg = Some(ReplicaArg::Load(store.load(&collection, &scope).unwrap()));
            }
            ReplicaCoroutineState::Yielded(ReplicaYield::WantsWrite(ops)) => {
                store.write(ops).unwrap();
                arg = Some(ReplicaArg::Write);
            }
            ReplicaCoroutineState::Yielded(other) => panic!("unexpected yield: {other:?}"),
            ReplicaCoroutineState::Complete(result) => {
                result.unwrap();
                return;
            }
        }
    }
}

fn inbox() -> ReplicaCollectionId {
    ReplicaCollectionId("INBOX".into())
}

/// A hydrated, linked placement with a matching base (so it projects clean).
fn placement(handle: &str, link: &str, hash: &str, flags: &[&str]) -> ReplicaPlacement {
    let flags = ReplicaFlags::from_iter(flags.iter().copied());
    ReplicaPlacement {
        sort_key: Default::default(),
        collection: inbox(),
        handle: ReplicaHandle(handle.into()),
        link_id: Some(ReplicaLinkId(link.into())),
        object: Some(ReplicaHash(hash.into())),
        level: ReplicaLevel::Full,
        meta: None,
        flags: flags.clone(),
        status: ReplicaStatus::Clean,
        conflict_revision: None,
        base: Some(ReplicaBase {
            flags,
            revision: None,
            object: Some(ReplicaHash(hash.into())),
        }),
        origin: None,
        ambiguous_handles: Vec::new(),
    }
}

/// A `Meta`-tier linked placement with no body: a summary is known, the
/// object is not local yet.
fn meta_placement(handle: &str, link: &str, meta: &str) -> ReplicaPlacement {
    ReplicaPlacement {
        sort_key: Default::default(),
        collection: inbox(),
        handle: ReplicaHandle(handle.into()),
        link_id: Some(ReplicaLinkId(link.into())),
        object: None,
        level: ReplicaLevel::Meta,
        meta: Some(ReplicaMeta(meta.into())),
        flags: ReplicaFlags::default(),
        status: ReplicaStatus::Clean,
        conflict_revision: None,
        base: Some(ReplicaBase {
            flags: ReplicaFlags::default(),
            revision: None,
            object: None,
        }),
        origin: None,
        ambiguous_handles: Vec::new(),
    }
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

fn blob_exists(dir: &Path, hash: &str) -> bool {
    dir.join("objects")
        .join(&hash[0..2])
        .join(&hash[2..4])
        .join(hash)
        .exists()
}

#[test]
fn single_source_write_reopen_lookup_and_gc() {
    let dir = tempfile::tempdir().unwrap();

    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");
    store
        .write(vec![
            store_object("cafebabe", b"abc"),
            ReplicaWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe", &["\\Seen"])),
            ReplicaWriteOp::SetCheckpoint {
                collection: inbox(),
                checkpoint: ReplicaCheckpoint(vec![1, 2, 3]),
            },
        ])
        .unwrap();
    assert!(blob_exists(dir.path(), "cafebabe"), "blob written");

    // reopen: the item, its flags, its body and the checkpoint survive
    drop(store);
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");
    let loaded = store.load(&inbox(), &ReplicaLoadScope::All).unwrap();
    assert_eq!(loaded.placements.len(), 1);
    assert_eq!(
        loaded.placements[0].object,
        Some(ReplicaHash("cafebabe".into()))
    );
    assert!(loaded.placements[0].flags.contains("\\Seen"));
    assert_eq!(loaded.checkpoint, Some(ReplicaCheckpoint(vec![1, 2, 3])));

    let known = store
        .lookup_objects(&[ReplicaLinkId("mid:a".into())])
        .unwrap();
    assert_eq!(
        known.get(&ReplicaLinkId("mid:a".into())),
        Some(&ReplicaHash("cafebabe".into()))
    );

    // dropping the only binding retires the item: gone from the seam,
    // kept in the trash with its body
    store
        .write(vec![ReplicaWriteOp::DropPlacement {
            collection: inbox(),
            handle: ReplicaHandle("1".into()),
            reason: ReplicaDropReason::Deleted,
        }])
        .unwrap();
    assert!(
        store
            .load(&inbox(), &ReplicaLoadScope::All)
            .unwrap()
            .placements
            .is_empty()
    );
    let retained = store.list_retained(&inbox(), None, 10).unwrap();
    assert_eq!(retained.len(), 1);
    assert!(blob_exists(dir.path(), "cafebabe"), "the body is kept");

    // purging is what orphans the blob, and it stays until a collector
    // runs
    assert!(store.purge(&inbox(), retained[0].seq).unwrap());
    assert!(blob_exists(dir.path(), "cafebabe"), "no write collects");

    let collected = store.collect_garbage().unwrap();
    assert_eq!((collected.objects, collected.blobs), (1, 1));
    assert!(
        !blob_exists(dir.path(), "cafebabe"),
        "orphan blob collected"
    );
}

#[test]
fn collection_kind_is_declared_and_survives_a_lazy_write() {
    let dir = tempfile::tempdir().unwrap();
    let store = PimdirStore::open(dir.path()).unwrap().for_source("local");

    // unknown until declared
    assert_eq!(store.collection_kind("INBOX").unwrap(), None);

    // declaring creates the collection with its media type
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    assert_eq!(
        store.collection_kind("INBOX").unwrap().as_deref(),
        Some("message/rfc822")
    );

    // a write's lazy `ENSURE_COLLECTION` must not clobber the kind
    let mut store = store;
    store
        .write(vec![ReplicaWriteOp::SetCheckpoint {
            collection: inbox(),
            checkpoint: ReplicaCheckpoint(vec![9]),
        }])
        .unwrap();
    assert_eq!(
        store.collection_kind("INBOX").unwrap().as_deref(),
        Some("message/rfc822"),
        "a sync write preserves the declared kind"
    );

    // redeclaring updates it
    store.ensure_collection("INBOX", "text/vcard").unwrap();
    assert_eq!(
        store.collection_kind("INBOX").unwrap().as_deref(),
        Some("text/vcard")
    );
}

#[test]
fn two_source_copy_and_delete_propagation() {
    let dir = tempfile::tempdir().unwrap();
    let mut left = PimdirStore::open(dir.path()).unwrap().for_source("left");
    let mut right = PimdirStore::open(dir.path()).unwrap().for_source("right");

    // the item lands on the left source only
    left.write(vec![
        store_object("cafebabe", b"abc"),
        ReplicaWriteOp::UpsertPlacement(placement("L1", "mid:a", "cafebabe", &["\\Seen"])),
    ])
    .unwrap();

    // right does not hold it yet, but the body is hydrated, so it
    // projects a Created copy
    let right_view = right.load(&inbox(), &ReplicaLoadScope::All).unwrap();
    assert_eq!(right_view.placements.len(), 1);
    assert_eq!(right_view.placements[0].status, ReplicaStatus::Created);
    assert_eq!(
        right_view.placements[0].object,
        Some(ReplicaHash("cafebabe".into()))
    );

    // right accepts the copy, binding its own handle
    right
        .write(vec![ReplicaWriteOp::UpsertPlacement(placement(
            "R1",
            "mid:a",
            "cafebabe",
            &["\\Seen"],
        ))])
        .unwrap();

    // left deletes it, and the item lingers because right still holds it
    left.write(vec![ReplicaWriteOp::DropPlacement {
        collection: inbox(),
        handle: ReplicaHandle("L1".into()),
        reason: ReplicaDropReason::Deleted,
    }])
    .unwrap();

    // right now sees a tombstone: the delete propagated as a pending
    // remove
    let right_view = right.load(&inbox(), &ReplicaLoadScope::All).unwrap();
    assert_eq!(right_view.placements.len(), 1);
    assert_eq!(right_view.placements[0].status, ReplicaStatus::Tombstone);
    // left no longer holds it, so it projects nothing rather than a
    // re-copy
    assert!(
        left.load(&inbox(), &ReplicaLoadScope::All)
            .unwrap()
            .placements
            .is_empty()
    );
}

#[test]
fn client_reads_collections_items_page_and_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();

    // three live items, out of link-id order on write
    store
        .write(vec![
            store_object("cafebabe", b"a"),
            store_object("cafebabf", b"b"),
            store_object("cafebac0", b"c"),
            ReplicaWriteOp::UpsertPlacement(placement("3", "mid:c", "cafebac0", &[])),
            ReplicaWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe", &["\\Seen"])),
            ReplicaWriteOp::UpsertPlacement(placement("2", "mid:b", "cafebabf", &[])),
        ])
        .unwrap();

    // list_collections surfaces the declared kind
    let collections = store.list_collections().unwrap();
    assert_eq!(collections.len(), 1);
    assert_eq!(collections[0].id, "INBOX");
    assert_eq!(collections[0].kind, "message/rfc822");

    // keyset pagination: ordered by link_id, `after` exclusive
    let page1 = store.list_items("INBOX", None, 2).unwrap();
    assert_eq!(
        page1
            .iter()
            .map(|i| i.link_id.0.as_str())
            .collect::<Vec<_>>(),
        ["mid:a", "mid:b"]
    );
    let cursor = page1.last().unwrap().link_id.0.clone();
    let page2 = store.list_items("INBOX", Some(&cursor), 2).unwrap();
    assert_eq!(
        page2
            .iter()
            .map(|i| i.link_id.0.as_str())
            .collect::<Vec<_>>(),
        ["mid:c"]
    );

    // full items carry their object and level, and flags round-trip
    assert_eq!(page1[0].level, ReplicaLevel::Full);
    assert_eq!(page1[0].object, Some(ReplicaHash("cafebabe".into())));
    assert!(page1[0].flags.contains("\\Seen"));

    // every item carries a public id, monotonic in insert order
    assert_eq!(
        store.get_item("INBOX", page1[0].seq).unwrap().unwrap().seq,
        page1[0].seq
    );

    // get_item keys on the public seq: a hit resolves to the link id, a
    // miss is None
    let seq_b = page1[1].seq; // mid:b
    assert_eq!(
        store.get_item("INBOX", seq_b).unwrap().unwrap().link_id.0,
        "mid:b"
    );
    assert!(store.get_item("INBOX", 9999).unwrap().is_none());

    assert_eq!(store.count_items("INBOX").unwrap(), 3);
}

#[test]
fn client_read_surfaces_level_for_a_meta_only_item() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");

    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(meta_placement(
            "1",
            "mid:a",
            "{\"v\":1,\"subject\":\"hi\"}",
        ))])
        .unwrap();

    // the only item in a fresh collection gets public seq 1
    let item = store.get_item("INBOX", 1).unwrap().unwrap();
    // availability-aware: a hydrate is needed, and the caller can tell
    // without touching the blob store
    assert_eq!(item.level, ReplicaLevel::Meta);
    assert!(item.object.is_none());
    assert_eq!(item.meta.unwrap().0, "{\"v\":1,\"subject\":\"hi\"}");
}

#[test]
fn public_seq_is_message_scoped_global_and_never_reused() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");
    for collection in ["INBOX", "Archives", "Sent"] {
        store
            .ensure_collection(collection, "message/rfc822")
            .unwrap();
    }

    // two messages in INBOX
    store
        .write(vec![
            ReplicaWriteOp::UpsertPlacement(meta_placement("1", "mid:a", "{\"v\":1}")),
            ReplicaWriteOp::UpsertPlacement(meta_placement("2", "mid:b", "{\"v\":1}")),
        ])
        .unwrap();
    let seq = |store: &PimdirStore, coll: &str, link: &str| {
        store
            .list_items(coll, None, 100)
            .unwrap()
            .into_iter()
            .find(|i| i.link_id.0 == link)
            .map(|i| i.seq)
    };
    let a = seq(&store, "INBOX", "mid:a").unwrap();
    let b = seq(&store, "INBOX", "mid:b").unwrap();
    assert!(b > a, "store-global and monotonic: {a} then {b}");

    // the same message filed in another mailbox keeps the same id: the id
    // belongs to the message, not the placement
    let mut a_in_arch = meta_placement("A1", "mid:a", "{\"v\":1}");
    a_in_arch.collection = ReplicaCollectionId("Archives".into());
    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(a_in_arch)])
        .unwrap();
    assert_eq!(
        seq(&store, "Archives", "mid:a"),
        Some(a),
        "a message keeps one id across every mailbox it is filed in"
    );
    // and `seq_for_link` agrees from either collection
    assert_eq!(store.seq_for_link("Archives", "mid:a").unwrap(), Some(a));

    // a different message draws the next store-global id: ids do not
    // restart per mailbox, so they never clash
    let mut s = meta_placement("S1", "mid:s", "{\"v\":1}");
    s.collection = ReplicaCollectionId("Sent".into());
    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(s)])
        .unwrap();
    let s_seq = seq(&store, "Sent", "mid:s").unwrap();
    assert!(
        s_seq > b,
        "a new message takes the next global id: {s_seq} > {b}"
    );

    // retire `mid:b`, then add a new message: b's id is never reused, and
    // a revive keeps the id it retired with
    store
        .write(vec![ReplicaWriteOp::DropPlacement {
            collection: inbox(),
            handle: ReplicaHandle("2".into()),
            reason: ReplicaDropReason::Deleted,
        }])
        .unwrap();
    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(meta_placement(
            "3",
            "mid:c",
            "{\"v\":1}",
        ))])
        .unwrap();
    let c = seq(&store, "INBOX", "mid:c").unwrap();
    assert!(
        c > s_seq,
        "a dropped message's id is never reused: next is {c}"
    );
}

#[test]
fn client_read_excludes_tombstones() {
    let dir = tempfile::tempdir().unwrap();
    let mut left = PimdirStore::open(dir.path()).unwrap().for_source("left");
    let mut right = PimdirStore::open(dir.path()).unwrap().for_source("right");

    // two items on left, only `mid:a` also bound on right
    left.write(vec![
        store_object("cafebabe", b"a"),
        store_object("cafebabf", b"b"),
        ReplicaWriteOp::UpsertPlacement(placement("LA", "mid:a", "cafebabe", &[])),
        ReplicaWriteOp::UpsertPlacement(placement("LB", "mid:b", "cafebabf", &[])),
    ])
    .unwrap();
    right.load(&inbox(), &ReplicaLoadScope::All).unwrap();
    right
        .write(vec![ReplicaWriteOp::UpsertPlacement(placement(
            "RA",
            "mid:a",
            "cafebabe",
            &[],
        ))])
        .unwrap();

    // left drops `mid:a` and right still holds it, so it lingers as a
    // tombstone while `mid:b` stays live
    left.write(vec![ReplicaWriteOp::DropPlacement {
        collection: inbox(),
        handle: ReplicaHandle("LA".into()),
        reason: ReplicaDropReason::Deleted,
    }])
    .unwrap();

    // the client read excludes the tombstone selectively
    let items = left.list_items("INBOX", None, 10).unwrap();
    assert_eq!(
        items
            .iter()
            .map(|i| i.link_id.0.as_str())
            .collect::<Vec<_>>(),
        ["mid:b"]
    );
    assert_eq!(left.count_items("INBOX").unwrap(), 1);
    // `mid:a`, public seq 1, is a tombstone and excluded from the read
    assert!(left.get_item("INBOX", 1).unwrap().is_none());
}

#[test]
fn a_body_streams_in_and_back_out() {
    use std::io::{Read, Write};

    let dir = tempfile::tempdir().unwrap();
    let blobs = io_pimdir::PimdirBlobs::open(dir.path(), PimdirHashAlgo::default());

    // stream a body in, in chunks, committing under its hash
    let mut w = blobs.writer().unwrap();
    w.write_all(b"hello ").unwrap();
    w.write_all(b"world").unwrap();
    let size = w.commit(&ReplicaHash("aabbccdd".into())).unwrap();
    assert_eq!(size, 11);
    assert!(
        blob_exists(dir.path(), "aabbccdd"),
        "committed to the shard path"
    );

    // read it back as a stream and buffered: both see the same bytes
    let mut file = blobs
        .reader(&ReplicaHash("aabbccdd".into()))
        .unwrap()
        .unwrap();
    let mut streamed = Vec::new();
    file.read_to_end(&mut streamed).unwrap();
    assert_eq!(streamed, b"hello world");
    assert_eq!(
        blobs.get(&ReplicaHash("aabbccdd".into())).unwrap().unwrap(),
        b"hello world"
    );

    // a missing object streams as None
    assert!(
        blobs
            .reader(&ReplicaHash("00000000".into()))
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_byteless_store_object_indexes_a_streamed_blob() {
    use std::io::Write;

    let dir = tempfile::tempdir().unwrap();
    let blobs = io_pimdir::PimdirBlobs::open(dir.path(), PimdirHashAlgo::default());
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");

    // the consumer streams the blob into place during a fetch
    let mut w = blobs.writer().unwrap();
    w.write_all(b"streamed-body").unwrap();
    let size = w.commit(&ReplicaHash("beef0000".into())).unwrap() as usize;

    // then the write batch records the object with no bytes, plus a
    // placement pointing at it
    store
        .write(vec![
            ReplicaWriteOp::StoreObject {
                object: ReplicaObject {
                    hash: ReplicaHash("beef0000".into()),
                    size,
                },
                body: None,
            },
            ReplicaWriteOp::UpsertPlacement(placement("1", "mid:a", "beef0000", &["\\Seen"])),
        ])
        .unwrap();

    // reopen: the object survived indexing, its blob is the streamed
    // bytes, and a placement references it so nothing collected it
    drop(store);
    let store = PimdirStore::open(dir.path()).unwrap().for_source("local");
    let loaded = store.load(&inbox(), &ReplicaLoadScope::All).unwrap();
    assert_eq!(
        loaded.placements[0].object,
        Some(ReplicaHash("beef0000".into()))
    );
    assert_eq!(
        blobs.get(&ReplicaHash("beef0000".into())).unwrap().unwrap(),
        b"streamed-body"
    );
}

#[test]
fn a_shared_blob_survives_until_its_last_referrer_is_purged() {
    // two items pointing at one content hash hold it with a refcount
    // above 1, so losing one keeps the blob and only the last release
    // collects it. Retention adds a second holder: a retired row pins its
    // body exactly as a live one does, so both must be purged.
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");

    store
        .write(vec![
            store_object("cafebabe", b"shared body"),
            ReplicaWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe", &["\\Seen"])),
            ReplicaWriteOp::UpsertPlacement(placement("2", "mid:b", "cafebabe", &["\\Seen"])),
        ])
        .unwrap();
    assert!(blob_exists(dir.path(), "cafebabe"), "shared blob written");

    // drop the first referrer: it is retired, and the blob stays for both
    // the second item and the retained row
    store
        .write(vec![ReplicaWriteOp::DropPlacement {
            collection: inbox(),
            handle: ReplicaHandle("1".into()),
            reason: ReplicaDropReason::Deleted,
        }])
        .unwrap();
    assert_eq!(
        store
            .load(&inbox(), &ReplicaLoadScope::All)
            .unwrap()
            .placements
            .len(),
        1
    );
    assert!(
        blob_exists(dir.path(), "cafebabe"),
        "blob kept while a second item still references it"
    );

    // drop the last live referrer: nothing is live any more, yet both
    // retained rows still pin the body
    store
        .write(vec![ReplicaWriteOp::DropPlacement {
            collection: inbox(),
            handle: ReplicaHandle("2".into()),
            reason: ReplicaDropReason::Deleted,
        }])
        .unwrap();
    assert!(
        store
            .load(&inbox(), &ReplicaLoadScope::All)
            .unwrap()
            .placements
            .is_empty()
    );
    let retained = store.list_retained(&inbox(), None, 10).unwrap();
    assert_eq!(retained.len(), 2);
    assert!(blob_exists(dir.path(), "cafebabe"), "both rows pin it");

    // purging the first releases one reference, and the blob outlives it
    assert!(store.purge(&inbox(), retained[0].seq).unwrap());
    assert!(
        blob_exists(dir.path(), "cafebabe"),
        "blob kept while the second retained row still references it"
    );

    // purging the last orphans it, and the collector unlinks it
    let report = store.purge(&inbox(), retained[1].seq).unwrap();
    assert!(report);
    assert!(blob_exists(dir.path(), "cafebabe"), "no purge collects");

    store.collect_garbage().unwrap();
    assert!(
        !blob_exists(dir.path(), "cafebabe"),
        "blob collected once its last referrer is purged"
    );
}

#[test]
fn a_flag_only_update_keeps_the_body() {
    // a flag change updates the item row in place and must not disturb
    // the object refcount, so the body stays put
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");

    store
        .write(vec![
            store_object("cafebabe", b"abc"),
            ReplicaWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe", &["\\Seen"])),
        ])
        .unwrap();

    // re-upsert the same item with a different flag set
    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(placement(
            "1",
            "mid:a",
            "cafebabe",
            &["\\Seen", "\\Flagged"],
        ))])
        .unwrap();

    let loaded = store.load(&inbox(), &ReplicaLoadScope::All).unwrap();
    assert_eq!(loaded.placements.len(), 1);
    assert!(loaded.placements[0].flags.contains("\\Flagged"));
    assert_eq!(
        loaded.placements[0].object,
        Some(ReplicaHash("cafebabe".into()))
    );
    assert!(
        blob_exists(dir.path(), "cafebabe"),
        "a flag-only change leaves the body intact"
    );
}

#[test]
fn a_staged_remove_hides_the_item_but_keeps_it_pending_a_sync() {
    // a client-staged Remove drops the item from the read surface yet
    // keeps the binding, so the next sync still pushes the expunge
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("right");
    store
        .write(vec![
            store_object("cafebabe", b"abc"),
            ReplicaWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe", &["\\Seen"])),
        ])
        .unwrap();
    assert_eq!(store.count_items("INBOX").unwrap(), 1);

    run_mutation(
        &mut store,
        "INBOX",
        ReplicaMutation::Remove(ReplicaHandle("1".into())),
    );

    // gone from the live-only client read surface
    assert_eq!(store.count_items("INBOX").unwrap(), 0);
    assert!(store.list_items("INBOX", None, 100).unwrap().is_empty());

    // but still projected as a Tombstone on reopen, so the next sync
    // pushes the remove against the kept remote handle
    let reopened = PimdirStore::open(dir.path()).unwrap().for_source("right");
    let projected = reopened
        .load(&inbox(), &ReplicaLoadScope::All)
        .unwrap()
        .placements;
    assert_eq!(projected.len(), 1, "the binding is kept for the sync");
    assert_eq!(projected[0].status, ReplicaStatus::Tombstone);
    assert!(
        projected[0].base.is_some(),
        "based, so the sync pushes a remove"
    );
}

#[test]
fn a_staged_move_empties_the_source_and_fills_the_target() {
    // a move stages a target create and a source tombstone: the source
    // empties, the target gains the item under the same message-scoped
    // seq, and each side projects the push its next sync derives
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("right");
    store
        .write(vec![
            store_object("cafebabe", b"abc"),
            ReplicaWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe", &["\\Seen"])),
        ])
        .unwrap();
    let seq = store.list_items("INBOX", None, 100).unwrap()[0].seq;

    run_mutation(
        &mut store,
        "INBOX",
        ReplicaMutation::Move {
            handle: ReplicaHandle("1".into()),
            target: ReplicaCollectionId("Archive".into()),
            placeholder: ReplicaHandle("tmp-1".into()),
        },
    );

    // the source empties and the target gains it, sharing the seq
    assert_eq!(store.count_items("INBOX").unwrap(), 0);
    let archived = store.list_items("Archive", None, 100).unwrap();
    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].link_id.0, "mid:a");
    assert_eq!(
        archived[0].seq, seq,
        "the moved message keeps its public id"
    );

    // the source projects a Tombstone and the target a base-less pending
    // push, both derived on the next sync
    let inbox_proj = store
        .load(&inbox(), &ReplicaLoadScope::All)
        .unwrap()
        .placements;
    assert_eq!(inbox_proj.len(), 1);
    assert_eq!(inbox_proj[0].status, ReplicaStatus::Tombstone);
    let archive_proj = store
        .load(
            &ReplicaCollectionId("Archive".into()),
            &ReplicaLoadScope::All,
        )
        .unwrap()
        .placements;
    assert_eq!(archive_proj.len(), 1);
    assert_ne!(
        archive_proj[0].status,
        ReplicaStatus::Clean,
        "a pending push"
    );
    assert!(
        archive_proj[0].base.is_none(),
        "no base yet: the sync appends it"
    );
}

#[test]
fn an_unknown_flag_set_stores_as_null_and_loads_back_unknown() {
    // spec §13 keeps the two absences apart: NULL means nothing has read
    // the markers, '[]' that the item carries none. A probed placement
    // storing '[]' would claim the second while only the first is true.
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");

    let mut probed = placement("1", "mid:a", "cafebabe", &[]);
    probed.flags = ReplicaFlags::Unknown;
    probed.base = None;
    let known_empty = placement("2", "mid:b", "cafebabe", &[]);

    store
        .write(vec![
            store_object("cafebabe", b"abc"),
            ReplicaWriteOp::UpsertPlacement(probed),
            ReplicaWriteOp::UpsertPlacement(known_empty),
        ])
        .unwrap();

    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    let stored: Vec<Option<String>> = conn
        .prepare("SELECT flags FROM items ORDER BY link_id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(stored, vec![None, Some(String::from("[]"))]);
    drop(conn);

    let loaded = store.load(&inbox(), &ReplicaLoadScope::All).unwrap();
    let mut placements = loaded.placements;
    placements.sort_by_key(|p| p.handle.0.clone());
    assert!(placements[0].flags.is_unknown());
    assert_eq!(placements[1].flags, ReplicaFlags::default());
}

#[test]
fn the_store_declares_the_hash_it_names_objects_by() {
    // a store records its algorithm once, at creation (spec §4.3), and a
    // handle computing a different name writes blobs no other reader
    // finds, failing as a dedup that never dedups
    let dir = tempfile::tempdir().unwrap();

    let store = PimdirStore::open_with_hash(dir.path(), Some(PimdirHashAlgo::Sha256_128)).unwrap();
    assert_eq!(store.hash_algo(), PimdirHashAlgo::Sha256_128);
    assert_eq!(store.hash(b"body").0.len(), 26);
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    let stored: String = conn
        .query_row("SELECT hash_algo FROM store_meta WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(stored, "sha256-128");
    drop(conn);

    // reopening adopts what the store records, and declaring another is
    // refused rather than renaming every body written from now on
    let reopened = PimdirStore::open(dir.path()).unwrap();
    assert_eq!(reopened.hash_algo(), PimdirHashAlgo::Sha256_128);
    drop(reopened);

    assert!(matches!(
        PimdirStore::open_with_hash(dir.path(), Some(PimdirHashAlgo::Blake3)),
        Err(PimdirError::HashAlgo { .. })
    ));
}

#[test]
fn a_base_of_nothing_round_trips_as_a_base() {
    // a source reporting no revision, no body and markers nobody has read
    // still agreed, and that agreement is what tells a pending push from
    // a settled one. Inferring the base's presence from its three
    // nullable columns loses exactly this shape.
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");

    let mut placement = placement("1", "mid:a", "cafebabe", &[]);
    placement.object = None;
    placement.level = ReplicaLevel::Probed;
    placement.flags = ReplicaFlags::Unknown;
    placement.base = Some(ReplicaBase {
        flags: ReplicaFlags::Unknown,
        revision: None,
        object: None,
    });
    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(placement)])
        .unwrap();

    let loaded = store
        .load(&inbox(), &ReplicaLoadScope::All)
        .unwrap()
        .placements;
    assert_eq!(loaded.len(), 1);
    let base = loaded[0]
        .base
        .as_ref()
        .expect("the agreement survives the round trip");
    assert!(base.flags.is_unknown());
    assert_eq!(loaded[0].status, ReplicaStatus::Clean, "not a pending push");
}
