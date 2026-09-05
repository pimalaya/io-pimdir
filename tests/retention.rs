//! Retention (spec §11): an item whose last source binding vanishes is
//! retained, not deleted, and only a purge takes it away.
//!
//! The load-hiding half is what makes it safe rather than a resurrection
//! loop, so the quiescence tests here run the real sync verb against a
//! fake source.

use std::{collections::BTreeMap, convert::Infallible, path::Path};

use io_pimdir::{
    change::{PimdirChange, PimdirDropReason, PimdirWriteOp},
    client::{PimdirError, PimdirStore},
    codec::PimdirAction,
    collection::{PimdirCheckpoint, PimdirCollectionId},
    load::PimdirLoadScope,
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
    remote::{
        PimdirFetchedBody, PimdirFetchedItem, PimdirPushResult, PimdirRemote, PimdirRemoteItem,
        PimdirRemoteSnapshot, PimdirTier,
    },
    sync::{PimdirSyncOptions, PimdirSyncReport},
};

fn inbox() -> PimdirCollectionId {
    PimdirCollectionId("INBOX".into())
}

/// A hydrated, linked placement with a matching base (so it projects clean).
fn placement(handle: &str, link: &str, hash: &str, flags: &[&str]) -> PimdirPlacement {
    let flags = PimdirFlags::from_iter(flags.iter().copied());
    PimdirPlacement {
        sort_key: Default::default(),
        collection: inbox(),
        handle: PimdirHandle(handle.into()),
        link_id: Some(PimdirLinkId(link.into())),
        object: Some(PimdirHash(hash.into())),
        level: PimdirLevel::Full,
        summary: None,
        flags: flags.clone(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags,
            revision: None,
            object: Some(PimdirHash(hash.into())),
        }),
        origin: None,
    }
}

fn store_object(hash: &str, body: &[u8]) -> PimdirWriteOp {
    PimdirWriteOp::StoreObject {
        object: PimdirObject {
            hash: PimdirHash(hash.into()),
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

fn drop_placement(handle: &str) -> PimdirWriteOp {
    PimdirWriteOp::DropPlacement {
        collection: inbox(),
        handle: PimdirHandle(handle.into()),
        reason: PimdirDropReason::Deleted,
    }
}

/// Overwrites a retained row's stamp, so a cutoff test does not depend on
/// the wall clock: a store aged in place is what the sweep meets.
fn backdate(dir: &Path, link: &str, retained_at: &str) {
    let conn = rusqlite::Connection::open(dir.join("pimdir.db")).unwrap();
    let updated = conn
        .execute(
            "UPDATE items SET retained_at = ?1 WHERE link_id = ?2 AND retained_at IS NOT NULL",
            rusqlite::params![retained_at, link],
        )
        .unwrap();
    assert_eq!(updated, 1, "{link} is retained");
}

#[test]
fn an_expunge_retains_the_item_and_its_body() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");
    store
        .write(vec![
            store_object("cafebabe", b"abc"),
            PimdirWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe", &["\\Seen"])),
        ])
        .unwrap();
    let seq = store.list_items("INBOX", None, 10).unwrap()[0].seq;

    // the source expunges it, so its last binding goes
    store.write(vec![drop_placement("1")]).unwrap();

    // gone from the sync seam and from the live reads
    assert!(
        store
            .load(&inbox(), &PimdirLoadScope::All)
            .unwrap()
            .placements
            .is_empty()
    );
    assert!(store.list_items("INBOX", None, 10).unwrap().is_empty());
    assert_eq!(store.count_items("INBOX").unwrap(), 0);
    assert!(store.get_item("INBOX", seq).unwrap().is_none());

    // but kept whole, body included, under the id it always had
    let retained = store.list_retained(inbox(), None, 10).unwrap();
    assert_eq!(retained.len(), 1);
    assert_eq!(retained[0].seq, seq);
    assert_eq!(retained[0].link_id.0, "mid:a");
    assert!(retained[0].flags.contains("\\Seen"));
    assert_eq!(retained[0].level, PimdirLevel::Full);
    assert_eq!(retained[0].object, Some(PimdirHash("cafebabe".into())));
    let retention = retained[0].retention.as_ref().expect("a retained row");
    assert_eq!(retention.size, Some(3));
    assert_eq!(retention.by.as_deref(), Some("local"));
    assert!(
        retention.at.ends_with('Z'),
        "an RFC 3339 stamp: {}",
        retention.at
    );
    assert_eq!(store.count_retained(inbox()).unwrap(), 1);
    assert_eq!(store.retained_bytes().unwrap(), 3);
    assert!(
        blob_exists(dir.path(), "cafebabe"),
        "the retained row pins its body against the sweep"
    );

    // it survives a reopen as retained, not as a live item
    drop(store);
    let store = PimdirStore::open(dir.path()).unwrap();
    assert_eq!(store.count_retained(inbox()).unwrap(), 1);
    assert_eq!(store.count_items("INBOX").unwrap(), 0);
}

#[test]
fn a_delta_and_a_full_resync_stay_quiescent_after_a_retention() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    let mut remote = MemRemote::default();
    remote.seed("1", "mid:a", b"body");

    store
        .sync("INBOX", PimdirSyncOptions::default(), &mut remote)
        .unwrap();
    // a sync enumerates handles only, and the hydrate resolves the link
    // id and the body, so the probe becomes a persisted item
    store
        .upgrade(
            "INBOX",
            vec![PimdirHandle("1".into())],
            PimdirTier::Full,
            &mut remote,
        )
        .unwrap();
    assert_eq!(store.count_items("INBOX").unwrap(), 1);

    // the source expunges the item and the sync observes the vanish
    remote.remove("1");
    let report = store
        .sync("INBOX", PimdirSyncOptions::default(), &mut remote)
        .unwrap();
    assert_eq!(report.pulled, 1, "the vanish is observed");
    assert_eq!(store.count_retained(inbox()).unwrap(), 1);

    // neither a delta nor a full resync re-derives against the hidden
    // row: the merge only sees what `load` returns
    let delta = store
        .sync("INBOX", PimdirSyncOptions::default(), &mut remote)
        .unwrap();
    assert_eq!(delta, PimdirSyncReport::default(), "quiescent delta sync");
    let full = store
        .sync(
            "INBOX",
            PimdirSyncOptions {
                full: true,
                ..Default::default()
            },
            &mut remote,
        )
        .unwrap();
    assert_eq!(full, PimdirSyncReport::default(), "quiescent full sync");

    // nothing was re-uploaded either, and the copy is still restorable
    assert!(remote.is_empty(), "no resurrection push");
    assert_eq!(store.count_retained(inbox()).unwrap(), 1);
    assert_eq!(store.count_items("INBOX").unwrap(), 0);
}

#[test]
fn a_reappearing_link_id_revives_the_retained_row() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");
    store
        .write(vec![
            store_object("cafebabe", b"abc"),
            PimdirWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe", &["\\Seen"])),
        ])
        .unwrap();
    let seq = store.list_items("INBOX", None, 10).unwrap()[0].seq;
    store.write(vec![drop_placement("1")]).unwrap();
    assert_eq!(store.count_retained(inbox()).unwrap(), 1);

    // the source hands the same link id back under a new handle, so the
    // retained row revives instead of colliding on the key
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(placement(
            "9",
            "mid:a",
            "cafebabe",
            &["\\Flagged"],
        ))])
        .unwrap();

    let items = store.list_items("INBOX", None, 10).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].seq, seq, "a restored item keeps its public id");
    assert!(items[0].flags.contains("\\Flagged"), "new content adopted");
    assert!(store.list_retained(inbox(), None, 10).unwrap().is_empty());
    assert_eq!(store.retained_bytes().unwrap(), 0);
    assert!(blob_exists(dir.path(), "cafebabe"));

    // the pin hand-over was exact: retiring it again keeps the body once
    // more, and purging then reclaims it
    store.write(vec![drop_placement("9")]).unwrap();
    assert!(blob_exists(dir.path(), "cafebabe"));
    assert!(store.purge(inbox(), seq).unwrap());
    assert_eq!(store.collect_garbage().unwrap().blobs, 1);
    assert!(!blob_exists(dir.path(), "cafebabe"), "no refcount leak");
}

#[test]
fn a_queued_add_restores_a_retained_item() {
    // restore is `Add` over the values retention preserved, with no new
    // action kind and no network, so the duplicate-link-id guard has to
    // exempt the retained row
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");
    store
        .write(vec![
            store_object("cafebabe", b"abc"),
            PimdirWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe", &["\\Seen"])),
        ])
        .unwrap();
    let seq = store.list_items("INBOX", None, 10).unwrap()[0].seq;
    store.write(vec![drop_placement("1")]).unwrap();

    let retained = store.list_retained(inbox(), None, 10).unwrap().remove(0);
    let mut producer =
        io_pimdir::client::producer::PimdirProducer::open(dir.path(), "pimdir").unwrap();
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::Add {
                link_id: Some(retained.link_id.clone()),
                flags: retained.flags.clone(),
                object: retained.object.clone(),
                handle: None,
            },
            None,
        )
        .unwrap();

    let report = store.drain_collection("INBOX").unwrap();
    assert_eq!((report.applied, report.parked, report.skipped), (1, 0, 0));

    let items = store.list_items("INBOX", None, 10).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].seq, seq, "restored under its own id");
    assert_eq!(items[0].object, Some(PimdirHash("cafebabe".into())));
    assert!(store.list_retained(inbox(), None, 10).unwrap().is_empty());

    // staged as a local creation, so the next sync pushes it back
    let projected = store
        .load(&inbox(), &PimdirLoadScope::All)
        .unwrap()
        .placements;
    assert_eq!(projected.len(), 1);
    assert_ne!(projected[0].status, PimdirStatus::Clean, "a pending push");
}

#[test]
fn purge_deletes_the_row_and_unlinks_the_body() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");
    store
        .write(vec![
            store_object("cafebabe", b"abc"),
            store_object("beef0000", b"defgh"),
            PimdirWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe", &[])),
            PimdirWriteOp::UpsertPlacement(placement("2", "mid:b", "beef0000", &[])),
        ])
        .unwrap();
    let live = store.list_items("INBOX", None, 10).unwrap();
    let (seq_a, seq_b) = (live[0].seq, live[1].seq);

    // a live item is out of a purge's reach entirely
    assert!(!store.purge(inbox(), seq_a).unwrap());
    assert_eq!(store.count_items("INBOX").unwrap(), 2);

    store.write(vec![drop_placement("1")]).unwrap();
    assert!(store.purge(inbox(), seq_a).unwrap());
    assert_eq!(store.count_retained(inbox()).unwrap(), 0);
    assert_eq!(store.collect_garbage().unwrap().blobs, 1);
    assert!(
        !blob_exists(dir.path(), "cafebabe"),
        "the last reference went with the row"
    );
    // the other item is untouched, body included
    assert_eq!(store.count_items("INBOX").unwrap(), 1);
    assert!(blob_exists(dir.path(), "beef0000"));
    assert!(store.get_item("INBOX", seq_b).unwrap().is_some());

    // purging what is already gone reports nothing to purge
    assert!(!store.purge(inbox(), seq_a).unwrap());
}

#[test]
fn purge_retained_before_respects_the_cutoff_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");
    store
        .write(vec![
            store_object("cafebabe", b"old"),
            store_object("beef0000", b"edge"),
            store_object("d0d00000", b"recent"),
            PimdirWriteOp::UpsertPlacement(placement("1", "mid:old", "cafebabe", &[])),
            PimdirWriteOp::UpsertPlacement(placement("2", "mid:edge", "beef0000", &[])),
            PimdirWriteOp::UpsertPlacement(placement("3", "mid:new", "d0d00000", &[])),
        ])
        .unwrap();
    store
        .write(vec![
            drop_placement("1"),
            drop_placement("2"),
            drop_placement("3"),
        ])
        .unwrap();

    const CUTOFF: &str = "2026-06-01T00:00:00.000Z";
    backdate(dir.path(), "mid:old", "2026-01-01T00:00:00.000Z");
    backdate(dir.path(), "mid:edge", CUTOFF);
    backdate(dir.path(), "mid:new", "2026-07-01T00:00:00.000Z");
    assert_eq!(store.retained_bytes().unwrap(), 13);

    // nothing is old enough for a cutoff before every stamp
    let report = store
        .purge_retained_before("2020-01-01T00:00:00.000Z")
        .unwrap();
    assert_eq!(report.items, 0);
    assert_eq!(store.count_retained(inbox()).unwrap(), 3);

    // strictly before: the item retired exactly at the cutoff is kept
    let report = store.purge_retained_before(CUTOFF).unwrap();
    assert_eq!(report.items, 1, "only the January one");
    let collected = store.collect_garbage().unwrap();
    assert_eq!((collected.blobs, collected.bytes), (1, 3));
    assert!(!blob_exists(dir.path(), "cafebabe"));
    let kept: Vec<String> = store
        .list_retained(inbox(), None, 10)
        .unwrap()
        .into_iter()
        .map(|item| item.link_id.0)
        .collect();
    assert_eq!(kept, ["mid:edge", "mid:new"]);
    assert!(blob_exists(dir.path(), "beef0000"));
    assert!(blob_exists(dir.path(), "d0d00000"));

    // a cutoff past every stamp empties the trash
    let report = store
        .purge_retained_before("2030-01-01T00:00:00.000Z")
        .unwrap();
    assert_eq!(report.items, 2);
    assert_eq!(store.count_retained(inbox()).unwrap(), 0);
    assert_eq!(store.retained_bytes().unwrap(), 0);
    let collected = store.collect_garbage().unwrap();
    assert_eq!((collected.blobs, collected.bytes), (2, 10));
    assert!(!blob_exists(dir.path(), "beef0000"));
    assert!(!blob_exists(dir.path(), "d0d00000"));
}

#[test]
fn a_two_side_delete_propagates_before_the_item_is_retired() {
    // retention is the terminal state of the `deleted` memory rather than
    // a shortcut past it: while another source holds the item, the
    // removal has to reach that source first
    let dir = tempfile::tempdir().unwrap();
    let mut left = PimdirStore::open(dir.path()).unwrap().for_source("left");
    let mut right = PimdirStore::open(dir.path()).unwrap().for_source("right");

    left.write(vec![
        store_object("cafebabe", b"abc"),
        PimdirWriteOp::UpsertPlacement(placement("L1", "mid:a", "cafebabe", &["\\Seen"])),
    ])
    .unwrap();
    right
        .write(vec![PimdirWriteOp::UpsertPlacement(placement(
            "R1",
            "mid:a",
            "cafebabe",
            &["\\Seen"],
        ))])
        .unwrap();

    // left's source expunged it and right must still be told, so the item
    // is a tombstone rather than retained
    left.write(vec![drop_placement("L1")]).unwrap();
    let projected = right
        .load(&inbox(), &PimdirLoadScope::All)
        .unwrap()
        .placements;
    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].status, PimdirStatus::Tombstone);
    assert_eq!(
        left.count_retained(inbox()).unwrap(),
        0,
        "the delete is still in flight"
    );

    // right pushes the remove and drops its own binding, so nothing holds
    // it
    right
        .write(vec![PimdirWriteOp::DropPlacement {
            collection: inbox(),
            handle: PimdirHandle("R1".into()),
            reason: PimdirDropReason::Deleted,
        }])
        .unwrap();
    assert!(
        right
            .load(&inbox(), &PimdirLoadScope::All)
            .unwrap()
            .placements
            .is_empty()
    );
    let retained = right.list_retained(inbox(), None, 10).unwrap();
    assert_eq!(retained.len(), 1);
    assert_eq!(
        retained[0]
            .retention
            .as_ref()
            .and_then(|retention| retention.by.as_deref()),
        Some("right"),
        "the source whose removal retired it"
    );
    assert!(blob_exists(dir.path(), "cafebabe"));
}

#[test]
fn the_retained_page_is_keyed_on_seq_and_exclusive() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");
    store
        .write(vec![
            store_object("cafebabe", b"shared"),
            PimdirWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe", &[])),
            PimdirWriteOp::UpsertPlacement(placement("2", "mid:b", "cafebabe", &[])),
            PimdirWriteOp::UpsertPlacement(placement("3", "mid:c", "cafebabe", &[])),
        ])
        .unwrap();
    store
        .write(vec![drop_placement("1"), drop_placement("3")])
        .unwrap();

    // the live item never shows up in the trash, whatever the page
    let page = store.list_retained(inbox(), None, 1).unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].link_id.0, "mid:a");
    let next = store.list_retained(inbox(), Some(page[0].seq), 10).unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].link_id.0, "mid:c");
    assert!(
        store
            .list_retained(inbox(), Some(next[0].seq), 10)
            .unwrap()
            .is_empty(),
        "the cursor is exclusive"
    );
}

/// A minimal fake source: it reports everything it holds, serves the
/// bodies and accepts every push. Enough to run a real sync end to end
/// and see whether a retained row provokes one.
#[derive(Default)]
struct MemRemote {
    items: BTreeMap<PimdirHandle, (PimdirLinkId, Vec<u8>)>,
}

impl MemRemote {
    fn seed(&mut self, handle: &str, link: &str, body: &[u8]) {
        self.items.insert(
            PimdirHandle(handle.into()),
            (PimdirLinkId(link.into()), body.to_vec()),
        );
    }

    fn remove(&mut self, handle: &str) {
        self.items.remove(&PimdirHandle(handle.into()));
    }

    fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// A stable, store-agnostic content hash for the fake bodies: the store
/// is hash-agnostic, and only stability matters here.
fn fake_hash(body: &[u8]) -> PimdirHash {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in body {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    PimdirHash(format!("{hash:016x}"))
}

impl PimdirRemote for MemRemote {
    type Error = Infallible;

    fn enumerate(
        &mut self,
        _collection: &PimdirCollectionId,
        _cursor: Option<PimdirCheckpoint>,
    ) -> Result<PimdirRemoteSnapshot, Infallible> {
        Ok(PimdirRemoteSnapshot {
            items: self
                .items
                .keys()
                .map(|handle| PimdirRemoteItem {
                    handle: handle.clone(),
                    flags: PimdirFlags::default(),
                    revision: None,
                })
                .collect(),
            vanished: Vec::new(),
            complete: true,
            checkpoint: PimdirCheckpoint(vec![self.items.len() as u8]),
        })
    }

    fn fetch(
        &mut self,
        _collection: &PimdirCollectionId,
        handles: Vec<PimdirHandle>,
        tier: PimdirTier,
    ) -> Result<Vec<PimdirFetchedItem>, Infallible> {
        Ok(handles
            .into_iter()
            .filter_map(|handle| {
                let (link, body) = self.items.get(&handle)?;
                Some(PimdirFetchedItem {
                    sort_key: Default::default(),
                    handle,
                    link_id: link.clone(),
                    summary: None,
                    body: matches!(tier, PimdirTier::Full).then(|| PimdirFetchedBody::Inline {
                        hash: fake_hash(body),
                        bytes: body.clone(),
                    }),
                    revision: None,
                })
            })
            .collect())
    }

    fn push(
        &mut self,
        _collection: &PimdirCollectionId,
        changes: Vec<PimdirChange>,
    ) -> Result<Vec<PimdirPushResult>, Infallible> {
        // NOTE: a retained item must provoke none of these, so a stray
        // push fails loudly rather than succeeding in silence.
        Ok(changes
            .into_iter()
            .map(|change| {
                panic!("unexpected push of {}", change.handle().0);
            })
            .collect())
    }
}

#[test]
fn a_handle_rebound_to_another_key_retires_the_item_it_held() {
    // STORAGE §9, §10: a hash: key names bytes, so a UID-less card edited
    // on its server changes key under the same DAV resource. The write
    // retires the old binding as a Deleted drop would, and the handle
    // names one item per source throughout.
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("dav");
    store
        .write(vec![
            store_object("cafebabe", b"BEGIN:VCARD\r\nFN:A\r\nEND:VCARD\r\n"),
            PimdirWriteOp::UpsertPlacement(placement("card.vcf", "hash:aaaa", "cafebabe", &[])),
        ])
        .unwrap();
    let old_seq = store.list_items("INBOX", None, 10).unwrap()[0].seq;

    store
        .write(vec![
            store_object("beef0000", b"BEGIN:VCARD\r\nFN:B\r\nEND:VCARD\r\n"),
            PimdirWriteOp::UpsertPlacement(placement("card.vcf", "hash:bbbb", "beef0000", &[])),
        ])
        .unwrap();

    // the new item is ordinary, under its own public id
    let live = store.list_items("INBOX", None, 10).unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].link_id.0, "hash:bbbb");
    assert_ne!(live[0].seq, old_seq, "a derived key draws its own seq");
    let projected = store
        .load(&inbox(), &PimdirLoadScope::All)
        .unwrap()
        .placements;
    assert_eq!(projected.len(), 1, "the handle names one item");
    assert_eq!(projected[0].link_id, Some(PimdirLinkId("hash:bbbb".into())));
    assert_eq!(projected[0].status, PimdirStatus::Clean);

    // the old one is retained with no binding, its body still pinned
    let retained = store.list_retained("INBOX", None, 10).unwrap();
    assert_eq!(retained.len(), 1);
    assert_eq!(retained[0].link_id.0, "hash:aaaa");
    assert_eq!(retained[0].seq, old_seq);
    assert!(
        store
            .item_bindings("INBOX", "hash:aaaa")
            .unwrap()
            .is_empty()
    );
    assert_eq!(store.collect_garbage().unwrap().blobs, 0);
    assert!(
        blob_exists(dir.path(), "cafebabe"),
        "retention pins the body"
    );

    // the other direction stays refused: the new key is bound to one
    // handle, and a second handle carrying it is a collision (§10)
    let collision = store.write(vec![PimdirWriteOp::UpsertPlacement(placement(
        "other.vcf",
        "hash:bbbb",
        "beef0000",
        &[],
    ))]);
    assert!(
        matches!(collision, Err(PimdirError::Rebind { .. })),
        "{collision:?}"
    );
}
