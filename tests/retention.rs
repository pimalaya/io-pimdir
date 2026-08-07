//! Retention (spec §16): an item whose last source binding vanishes is
//! retained, not deleted, and only a purge takes it away.
//!
//! The load-hiding half is what makes it safe rather than a resurrection loop,
//! so the quiescence tests here drive a **real** [`ReplicaClient`] against a
//! fake source, mirroring `io-replica/tests/soft_delete.rs`: the reference
//! implementation of the contract this store now satisfies.

use std::{collections::BTreeMap, convert::Infallible, path::Path};

use io_pimdir::{PimdirStore, codec::PimdirAction};
use io_replica::{
    change::{ReplicaChange, ReplicaWriteOp},
    client::{ReplicaClient, ReplicaRemote, ReplicaStorage},
    collection::{ReplicaCheckpoint, ReplicaCollectionId},
    object::{ReplicaHash, ReplicaObject},
    placement::{
        ReplicaBase, ReplicaFlags, ReplicaHandle, ReplicaLevel, ReplicaLinkId, ReplicaMeta,
        ReplicaPlacement, ReplicaStatus,
    },
    remote::{
        ReplicaFetchedBody, ReplicaFetchedItem, ReplicaPushResult, ReplicaRemoteItem,
        ReplicaRemoteSnapshot, ReplicaTier,
    },
    sync::{ReplicaSyncOptions, ReplicaSyncReport},
};

fn inbox() -> ReplicaCollectionId {
    ReplicaCollectionId("INBOX".into())
}

/// A hydrated, linked placement with a matching base (so it projects clean).
fn placement(handle: &str, link: &str, hash: &str, flags: &[&str]) -> ReplicaPlacement {
    let flags = ReplicaFlags::from_iter(flags.iter().copied());
    ReplicaPlacement {
        collection: inbox(),
        handle: ReplicaHandle(handle.into()),
        link_id: Some(ReplicaLinkId(link.into())),
        object: Some(ReplicaHash(hash.into())),
        level: ReplicaLevel::Full,
        meta: Some(ReplicaMeta("{\"v\":1}".into())),
        flags: flags.clone(),
        status: ReplicaStatus::Clean,
        conflict_revision: None,
        base: Some(ReplicaBase {
            flags,
            revision: None,
            object: Some(ReplicaHash(hash.into())),
        }),
        origin: None,
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

fn drop_placement(handle: &str) -> ReplicaWriteOp {
    ReplicaWriteOp::DropPlacement {
        collection: inbox(),
        handle: ReplicaHandle(handle.into()),
    }
}

/// Overwrites a retained row's stamp, so a cutoff test does not depend on the
/// wall clock: a store aged in place is exactly what the sweep meets.
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
    let mut store = PimdirStore::open(dir.path(), "local").unwrap();
    store
        .write(vec![
            store_object("cafebabe", b"abc"),
            ReplicaWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe", &["\\Seen"])),
        ])
        .unwrap();
    let seq = store.list_items("INBOX", None, 10).unwrap()[0].seq;

    // The source expunges it: its last (only) binding goes.
    store.write(vec![drop_placement("1")]).unwrap();

    // Gone from the sync seam and from the live reads...
    assert!(store.load(&inbox()).unwrap().placements.is_empty());
    assert!(store.list_items("INBOX", None, 10).unwrap().is_empty());
    assert_eq!(store.count_items("INBOX").unwrap(), 0);
    assert!(store.get_item("INBOX", seq).unwrap().is_none());

    // ...but kept whole, body included, under the id it always had.
    let retained = store.list_retained(&inbox(), None, 10).unwrap();
    assert_eq!(retained.len(), 1);
    assert_eq!(retained[0].seq, seq);
    assert_eq!(retained[0].link_id, "mid:a");
    assert!(retained[0].flags.contains("\\Seen"));
    assert_eq!(retained[0].level, ReplicaLevel::Full);
    assert_eq!(retained[0].meta.as_deref(), Some("{\"v\":1}"));
    assert_eq!(retained[0].object_hash.as_deref(), Some("cafebabe"));
    assert_eq!(retained[0].size, Some(3));
    assert_eq!(retained[0].retained_by.as_deref(), Some("local"));
    assert!(
        retained[0].retained_at.ends_with('Z'),
        "an RFC 3339 stamp: {}",
        retained[0].retained_at
    );
    assert_eq!(store.count_retained(&inbox()).unwrap(), 1);
    assert_eq!(store.retained_bytes().unwrap(), 3);
    assert!(
        blob_exists(dir.path(), "cafebabe"),
        "the retained row pins its body against the sweep"
    );

    // It survives a reopen as retained, not as a live item.
    drop(store);
    let store = PimdirStore::open(dir.path(), "local").unwrap();
    assert_eq!(store.count_retained(&inbox()).unwrap(), 1);
    assert_eq!(store.count_items("INBOX").unwrap(), 0);
}

#[test]
fn a_delta_and_a_full_resync_stay_quiescent_after_a_retention() {
    let dir = tempfile::tempdir().unwrap();
    let store = PimdirStore::open(dir.path(), "local").unwrap();
    let mut remote = MemRemote::default();
    remote.seed("1", "mid:a", b"body");
    let mut client = ReplicaClient::new(store, remote);

    client.sync("INBOX", ReplicaSyncOptions::default()).unwrap();
    // A sync enumerates handles only; the hydrate is what resolves the link id
    // and the body, so the probe becomes a persisted item.
    client
        .upgrade("INBOX", vec![ReplicaHandle("1".into())], ReplicaTier::Full)
        .unwrap();
    assert_eq!(client.storage().count_items("INBOX").unwrap(), 1);

    // The source expunges the item; the sync observes the vanish.
    client.remote_mut().remove("1");
    let report = client.sync("INBOX", ReplicaSyncOptions::default()).unwrap();
    assert_eq!(report.pulled, 1, "the vanish is observed");
    assert_eq!(client.storage().count_retained(&inbox()).unwrap(), 1);

    // Neither a delta nor a full resync re-derives against the hidden row: the
    // merge only ever sees what `load` returns.
    let delta = client.sync("INBOX", ReplicaSyncOptions::default()).unwrap();
    assert_eq!(delta, ReplicaSyncReport::default(), "quiescent delta sync");
    let full = client
        .sync(
            "INBOX",
            ReplicaSyncOptions {
                full: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(full, ReplicaSyncReport::default(), "quiescent full sync");

    // Nothing was re-uploaded either, and the copy is still there to restore.
    assert!(client.remote().is_empty(), "no resurrection push");
    assert_eq!(client.storage().count_retained(&inbox()).unwrap(), 1);
    assert_eq!(client.storage().count_items("INBOX").unwrap(), 0);
}

#[test]
fn a_reappearing_link_id_revives_the_retained_row() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path(), "local").unwrap();
    store
        .write(vec![
            store_object("cafebabe", b"abc"),
            ReplicaWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe", &["\\Seen"])),
        ])
        .unwrap();
    let seq = store.list_items("INBOX", None, 10).unwrap()[0].seq;
    store.write(vec![drop_placement("1")]).unwrap();
    assert_eq!(store.count_retained(&inbox()).unwrap(), 1);

    // The source hands the same link id back under a new handle (a
    // resurrection): the retained row revives instead of colliding on the key.
    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(placement(
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
    assert!(store.list_retained(&inbox(), None, 10).unwrap().is_empty());
    assert_eq!(store.retained_bytes().unwrap(), 0);
    assert!(blob_exists(dir.path(), "cafebabe"));

    // The pin hand-over was exact: retiring it again keeps the body once more,
    // and purging then reclaims it (no leaked reference either way).
    store.write(vec![drop_placement("9")]).unwrap();
    assert!(blob_exists(dir.path(), "cafebabe"));
    assert!(store.purge(&inbox(), seq).unwrap());
    assert!(!blob_exists(dir.path(), "cafebabe"), "no refcount leak");
}

#[test]
fn a_queued_add_restores_a_retained_item() {
    // Restore is `Add` over the values retention preserved (no new action kind,
    // no network): the duplicate-link-id guard must exempt the retained row.
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path(), "local").unwrap();
    store
        .write(vec![
            store_object("cafebabe", b"abc"),
            ReplicaWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe", &["\\Seen"])),
        ])
        .unwrap();
    let seq = store.list_items("INBOX", None, 10).unwrap()[0].seq;
    store.write(vec![drop_placement("1")]).unwrap();

    let retained = store.list_retained(&inbox(), None, 10).unwrap().remove(0);
    let mut producer = io_pimdir::PimdirProducer::open(dir.path(), "pimdir").unwrap();
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::Add {
                link_id: Some(ReplicaLinkId(retained.link_id.clone())),
                flags: retained.flags.clone(),
                object: retained.object_hash.clone().map(ReplicaHash),
                meta: retained.meta.clone().map(ReplicaMeta),
                handle: None,
            },
            None,
            "2026-08-07T00:00:00Z",
        )
        .unwrap();

    let report = store.drain_collection("INBOX").unwrap();
    assert_eq!((report.applied, report.parked, report.skipped), (1, 0, 0));

    let items = store.list_items("INBOX", None, 10).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].seq, seq, "restored under its own id");
    assert_eq!(items[0].object, Some(ReplicaHash("cafebabe".into())));
    assert!(store.list_retained(&inbox(), None, 10).unwrap().is_empty());

    // Staged as a local creation, so the next sync pushes it back to the source.
    let projected = store.load(&inbox()).unwrap().placements;
    assert_eq!(projected.len(), 1);
    assert_ne!(projected[0].status, ReplicaStatus::Clean, "a pending push");
}

#[test]
fn purge_deletes_the_row_and_unlinks_the_body() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path(), "local").unwrap();
    store
        .write(vec![
            store_object("cafebabe", b"abc"),
            store_object("beef0000", b"defgh"),
            ReplicaWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe", &[])),
            ReplicaWriteOp::UpsertPlacement(placement("2", "mid:b", "beef0000", &[])),
        ])
        .unwrap();
    let live = store.list_items("INBOX", None, 10).unwrap();
    let (seq_a, seq_b) = (live[0].seq, live[1].seq);

    // A live item is out of a purge's reach entirely.
    assert!(!store.purge(&inbox(), seq_a).unwrap());
    assert_eq!(store.count_items("INBOX").unwrap(), 2);

    store.write(vec![drop_placement("1")]).unwrap();
    assert!(store.purge(&inbox(), seq_a).unwrap());
    assert_eq!(store.count_retained(&inbox()).unwrap(), 0);
    assert!(
        !blob_exists(dir.path(), "cafebabe"),
        "the last reference went with the row"
    );
    // The other item is untouched, body included.
    assert_eq!(store.count_items("INBOX").unwrap(), 1);
    assert!(blob_exists(dir.path(), "beef0000"));
    assert!(store.get_item("INBOX", seq_b).unwrap().is_some());

    // Purging what is already gone reports nothing to purge.
    assert!(!store.purge(&inbox(), seq_a).unwrap());
}

#[test]
fn purge_retained_before_respects_the_cutoff_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path(), "local").unwrap();
    store
        .write(vec![
            store_object("cafebabe", b"old"),
            store_object("beef0000", b"edge"),
            store_object("d0d00000", b"recent"),
            ReplicaWriteOp::UpsertPlacement(placement("1", "mid:old", "cafebabe", &[])),
            ReplicaWriteOp::UpsertPlacement(placement("2", "mid:edge", "beef0000", &[])),
            ReplicaWriteOp::UpsertPlacement(placement("3", "mid:new", "d0d00000", &[])),
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

    // Nothing is old enough for a cutoff before every stamp.
    let report = store
        .purge_retained_before("2020-01-01T00:00:00.000Z")
        .unwrap();
    assert_eq!((report.items, report.bytes), (0, 0));
    assert_eq!(store.count_retained(&inbox()).unwrap(), 3);

    // Strictly before: the item retired exactly at the cutoff is kept.
    let report = store.purge_retained_before(CUTOFF).unwrap();
    assert_eq!((report.items, report.bytes), (1, 3), "only the January one");
    assert!(!blob_exists(dir.path(), "cafebabe"));
    let kept: Vec<String> = store
        .list_retained(&inbox(), None, 10)
        .unwrap()
        .into_iter()
        .map(|item| item.link_id)
        .collect();
    assert_eq!(kept, ["mid:edge", "mid:new"]);
    assert!(blob_exists(dir.path(), "beef0000"));
    assert!(blob_exists(dir.path(), "d0d00000"));

    // A cutoff past every stamp empties the trash.
    let report = store
        .purge_retained_before("2030-01-01T00:00:00.000Z")
        .unwrap();
    assert_eq!((report.items, report.bytes), (2, 10));
    assert_eq!(store.count_retained(&inbox()).unwrap(), 0);
    assert_eq!(store.retained_bytes().unwrap(), 0);
    assert!(!blob_exists(dir.path(), "beef0000"));
    assert!(!blob_exists(dir.path(), "d0d00000"));
}

#[test]
fn a_two_side_delete_propagates_before_the_item_is_retired() {
    // Retention is the terminal state of the `deleted` memory, not a shortcut
    // past it: while another source still holds the item, the removal has to
    // reach that source first.
    let dir = tempfile::tempdir().unwrap();
    let mut left = PimdirStore::open(dir.path(), "left").unwrap();
    let mut right = PimdirStore::open(dir.path(), "right").unwrap();

    left.write(vec![
        store_object("cafebabe", b"abc"),
        ReplicaWriteOp::UpsertPlacement(placement("L1", "mid:a", "cafebabe", &["\\Seen"])),
    ])
    .unwrap();
    right
        .write(vec![ReplicaWriteOp::UpsertPlacement(placement(
            "R1",
            "mid:a",
            "cafebabe",
            &["\\Seen"],
        ))])
        .unwrap();

    // Left's source expunged it: right must still be told, so the item is a
    // tombstone, NOT retained.
    left.write(vec![drop_placement("L1")]).unwrap();
    let projected = right.load(&inbox()).unwrap().placements;
    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].status, ReplicaStatus::Tombstone);
    assert_eq!(
        left.count_retained(&inbox()).unwrap(),
        0,
        "the delete is still in flight"
    );

    // Right pushes the remove and drops its own binding: now nothing holds it.
    right
        .write(vec![ReplicaWriteOp::DropPlacement {
            collection: inbox(),
            handle: ReplicaHandle("R1".into()),
        }])
        .unwrap();
    assert!(right.load(&inbox()).unwrap().placements.is_empty());
    let retained = right.list_retained(&inbox(), None, 10).unwrap();
    assert_eq!(retained.len(), 1);
    assert_eq!(
        retained[0].retained_by.as_deref(),
        Some("right"),
        "the source whose removal retired it"
    );
    assert!(blob_exists(dir.path(), "cafebabe"));
}

#[test]
fn the_retained_page_is_keyed_on_seq_and_exclusive() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path(), "local").unwrap();
    store
        .write(vec![
            store_object("cafebabe", b"shared"),
            ReplicaWriteOp::UpsertPlacement(placement("1", "mid:a", "cafebabe", &[])),
            ReplicaWriteOp::UpsertPlacement(placement("2", "mid:b", "cafebabe", &[])),
            ReplicaWriteOp::UpsertPlacement(placement("3", "mid:c", "cafebabe", &[])),
        ])
        .unwrap();
    store
        .write(vec![drop_placement("1"), drop_placement("3")])
        .unwrap();

    // The live item never shows up in the trash, whatever the page.
    let page = store.list_retained(&inbox(), None, 1).unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].link_id, "mid:a");
    let next = store
        .list_retained(&inbox(), Some(page[0].seq), 10)
        .unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].link_id, "mid:c");
    assert!(
        store
            .list_retained(&inbox(), Some(next[0].seq), 10)
            .unwrap()
            .is_empty(),
        "the cursor is exclusive"
    );
}

/// A minimal fake source: it reports everything it currently holds, serves the
/// bodies, and accepts every push. Enough to drive a real sync end to end and
/// see whether a retained row provokes one.
#[derive(Default)]
struct MemRemote {
    items: BTreeMap<ReplicaHandle, (ReplicaLinkId, Vec<u8>)>,
}

impl MemRemote {
    fn seed(&mut self, handle: &str, link: &str, body: &[u8]) {
        self.items.insert(
            ReplicaHandle(handle.into()),
            (ReplicaLinkId(link.into()), body.to_vec()),
        );
    }

    fn remove(&mut self, handle: &str) {
        self.items.remove(&ReplicaHandle(handle.into()));
    }

    fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// A stable, store-agnostic content hash for the fake bodies (the store is
/// hash-agnostic; only its stability matters here).
fn fake_hash(body: &[u8]) -> ReplicaHash {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in body {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    ReplicaHash(format!("{hash:016x}"))
}

impl ReplicaRemote for MemRemote {
    type Error = Infallible;

    fn enumerate(
        &mut self,
        _collection: &ReplicaCollectionId,
        _cursor: Option<ReplicaCheckpoint>,
    ) -> Result<ReplicaRemoteSnapshot, Infallible> {
        Ok(ReplicaRemoteSnapshot {
            items: self
                .items
                .keys()
                .map(|handle| ReplicaRemoteItem {
                    handle: handle.clone(),
                    flags: ReplicaFlags::default(),
                    revision: None,
                })
                .collect(),
            vanished: Vec::new(),
            complete: true,
            checkpoint: ReplicaCheckpoint(vec![self.items.len() as u8]),
        })
    }

    fn fetch(
        &mut self,
        _collection: &ReplicaCollectionId,
        handles: Vec<ReplicaHandle>,
        tier: ReplicaTier,
    ) -> Result<Vec<ReplicaFetchedItem>, Infallible> {
        Ok(handles
            .into_iter()
            .filter_map(|handle| {
                let (link, body) = self.items.get(&handle)?;
                Some(ReplicaFetchedItem {
                    handle,
                    link_id: link.clone(),
                    meta: ReplicaMeta("{\"v\":1}".into()),
                    body: matches!(tier, ReplicaTier::Full).then(|| ReplicaFetchedBody::Inline {
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
        _collection: &ReplicaCollectionId,
        changes: Vec<ReplicaChange>,
    ) -> Result<Vec<ReplicaPushResult>, Infallible> {
        // NOTE: a retained item must provoke none of these; the tests assert on
        // the report, and this keeps a stray push from silently succeeding.
        Ok(changes
            .into_iter()
            .map(|change| {
                let handle = match change {
                    ReplicaChange::Add { handle, .. }
                    | ReplicaChange::Remove { handle, .. }
                    | ReplicaChange::SetFlags { handle, .. }
                    | ReplicaChange::Update { handle, .. } => handle,
                };
                panic!("unexpected push of {}", handle.0);
            })
            .collect())
    }
}
