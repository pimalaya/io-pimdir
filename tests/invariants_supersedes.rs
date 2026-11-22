//! An add superseding an item holds its delete while the create it made
//! is pending (STORAGE §15.3, SYNC §5), ported from pimdir's retired
//! checks/invariants.sh: what holds the tombstone and what releases it.

#![cfg(feature = "client")]

mod invariants_common;

use std::path::Path;

use io_pimdir::{
    change::PimdirWriteOp,
    client::{PimdirSourceStore, PimdirStore, producer::PimdirProducer},
    codec::{PimdirAction, PimdirSupersedes},
    collection::{PimdirCheckpoint, PimdirCollectionId},
    load::PimdirLoadSelection,
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirPlacement, PimdirStatus,
    },
    summary,
};

use invariants_common::*;

const CAL: &str = "text/calendar";

fn task(uid: &str) -> Vec<u8> {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//x//x//EN\r\nBEGIN:VTODO\r\n\
         UID:{uid}\r\nDTSTAMP:20261101T100000Z\r\nSUMMARY:{uid}\r\nEND:VTODO\r\n\
         END:VCALENDAR\r\n"
    )
    .into_bytes()
}

/// A clean, based placement of the task `bytes` in `collection`.
fn placed(
    store: &PimdirSourceStore,
    collection: &str,
    handle: &str,
    bytes: &[u8],
) -> PimdirWriteOp {
    let derived = summary::derive(CAL, bytes).unwrap();
    let hash = store.hash(bytes);
    PimdirWriteOp::UpsertPlacement(PimdirPlacement {
        collection: PimdirCollectionId(collection.into()),
        handle: PimdirHandle(handle.into()),
        link_id: Some(derived.link_id),
        object: Some(hash.clone()),
        level: PimdirLevel::Full,
        summary: derived.summary,
        sort_key: derived.sort_key,
        flags: PimdirFlags::default(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: Some("r1".into()),
            object: Some(hash),
        }),
        origin: None,
        server_object_id: None,
    })
}

fn checkpoint(collection: &str) -> PimdirWriteOp {
    PimdirWriteOp::SetCheckpoint {
        collection: PimdirCollectionId(collection.into()),
        checkpoint: PimdirCheckpoint(b"c1".to_vec()),
    }
}

/// The handles of tasks-a whose remove a load holds.
fn held(store: &PimdirSourceStore) -> Vec<String> {
    store
        .load(
            &PimdirCollectionId("tasks-a".into()),
            &PimdirLoadSelection::All,
        )
        .unwrap()
        .held
        .into_iter()
        .map(|handle| handle.0)
        .collect()
}

/// `t1` bound in tasks-a by caldav, which syncs tasks-b too, and an add of
/// `n1` there superseding `t1`, drained.
fn superseded(dir: &Path) -> PimdirSourceStore {
    let mut store = PimdirStore::open(dir).unwrap().for_source("caldav");
    store.ensure_collection("tasks-a", CAL).unwrap();
    store.ensure_collection("tasks-b", CAL).unwrap();
    let t1 = task("t1");
    store
        .write(vec![
            body(&store, &t1),
            placed(&store, "tasks-a", "a.ics", &t1),
            checkpoint("tasks-a"),
            checkpoint("tasks-b"),
        ])
        .unwrap();
    let seq = store.seq_for_link("tasks-a", "t1").unwrap().unwrap();

    let mut producer = PimdirProducer::open(dir, "test").unwrap();
    producer
        .enqueue_with_body("tasks-b", &task("n1"), |object| PimdirAction::Add {
            link_id: None,
            flags: PimdirFlags::default(),
            object: Some(object),
            supersedes: Some(PimdirSupersedes {
                collection: PimdirCollectionId("tasks-a".into()),
                seq,
            }),
        })
        .unwrap();
    drop(producer);
    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (1, 0));
    store
}

/// L1652, L1649, L1655: a source syncing the superseding collection by its
/// checkpoint alone holds the tombstone; no binding and no checkpoint
/// there release it; the superseding create withdrawn releases it.
#[test]
fn l1652_l1649_l1655_what_holds_a_superseded_tombstone() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = superseded(dir.path());
    assert_eq!(held(&store), ["a.ics"], "the pending create holds it");

    let conn = peek(dir.path());
    conn.execute_batch(
        "DELETE FROM bindings WHERE collection = 'tasks-b';
         DELETE FROM sources WHERE collection = 'tasks-b';",
    )
    .unwrap();
    assert!(
        held(&store).is_empty(),
        "L1649: no binding and no checkpoint on tasks-b"
    );

    store.write(vec![checkpoint("tasks-b")]).unwrap();
    assert_eq!(held(&store), ["a.ics"], "L1652: held again");

    let n1 = store.seq_for_link("tasks-b", "n1").unwrap().unwrap();
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    producer
        .enqueue(
            "tasks-b",
            &PimdirAction::Remove {
                seq: n1,
                if_match: None,
                resolves: None,
            },
            None,
        )
        .unwrap();
    drop(producer);
    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (1, 0));
    assert_eq!(store.count_retained("tasks-b").unwrap(), 1);
    assert!(held(&store).is_empty(), "L1655: the create withdrawn");
}

/// L1661: a revived superseded item is no tombstone to hold.
#[test]
fn l1661_a_revived_item_is_not_held() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = superseded(dir.path());
    assert_eq!(held(&store), ["a.ics"]);

    let t1 = task("t1");
    store
        .write(vec![placed(&store, "tasks-a", "a.ics", &t1)])
        .unwrap();
    assert!(held(&store).is_empty());
}

/// L1643, L1665: nothing is held in the superseding collection, and a
/// rename of it carries the pair, the hold standing.
#[test]
fn l1643_l1665_the_hold_lives_in_the_superseded_collection() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = superseded(dir.path());
    let held_in_b = store
        .load(
            &PimdirCollectionId("tasks-b".into()),
            &PimdirLoadSelection::All,
        )
        .unwrap()
        .held;
    assert!(held_in_b.is_empty(), "L1643");

    store.rename_collection("tasks-b", "tasks-c").unwrap();
    assert_eq!(held(&store), ["a.ics"], "L1665");
}
