//! The queue's drain against `store.mutate` (pimdir SYNC §7, §10; STORAGE
//! §14): two sources on one store, `right` binding `mid:L` in T, `left`
//! binding the same message in S and syncing T too. A Copy or a Move of
//! left's item from S to T, drained from the queue or run through
//! `store.mutate`, leaves the same rows: left joins right's item under the
//! bare key, where the drain minted a second item before (K4 of pimdir
//! cairn/changes/a-smaller-sync). A source syncing no T stages its create
//! under one that does (pimdir SYNC §7).

#![cfg(feature = "client")]

use std::path::Path;

use io_pimdir::{
    change::PimdirWriteOp,
    client::{PimdirSourceStore, PimdirStore, producer::PimdirProducer},
    codec::PimdirAction,
    collection::{PimdirCheckpoint, PimdirCollectionId},
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

const BODY: &[u8] = b"Subject: mid:L\r\n\r\nthe shared message\r\n";

fn placement(collection: &str, handle: &str) -> PimdirPlacement {
    PimdirPlacement {
        sort_key: Default::default(),
        collection: PimdirCollectionId(collection.into()),
        handle: PimdirHandle(handle.into()),
        link_id: Some(PimdirLinkId("mid:L".into())),
        object: Some(hash::hash(BODY)),
        level: PimdirLevel::Full,
        summary: Some(PimdirSummary::Mail(PimdirMailSummary {
            subject: "mid:L".into(),
            ..Default::default()
        })),
        flags: PimdirFlags::default(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: None,
            object: Some(hash::hash(BODY)),
        }),
        origin: None,
        server_object_id: None,
    }
}

fn seeded(dir: &Path) -> PimdirSourceStore {
    let mut right = PimdirStore::open(dir).unwrap().for_source("right");
    right.ensure_collection("S", "message/rfc822").unwrap();
    right.ensure_collection("T", "message/rfc822").unwrap();
    right
        .write(vec![
            PimdirWriteOp::StoreObject {
                object: PimdirObject {
                    hash: hash::hash(BODY),
                    size: BODY.len(),
                },
                body: Some(BODY.to_vec()),
            },
            PimdirWriteOp::UpsertPlacement(placement("T", "t1")),
        ])
        .unwrap();
    drop(right);
    let mut left = PimdirStore::open(dir).unwrap().for_source("left");
    left.write(vec![
        PimdirWriteOp::UpsertPlacement(placement("S", "s1")),
        PimdirWriteOp::SetCheckpoint {
            collection: PimdirCollectionId("T".into()),
            checkpoint: PimdirCheckpoint(b"cp".to_vec()),
        },
    ])
    .unwrap();
    left
}

#[derive(Debug, PartialEq)]
struct Row {
    source: String,
    handle: String,
    link_id: String,
    seq: i64,
    status_base: i64,
}

fn target(dir: &Path) -> Vec<Row> {
    let conn = rusqlite::Connection::open(dir.join("pimdir.db")).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT b.source, b.handle, i.link_id, i.seq, b.base_present \
             FROM bindings b JOIN items i ON i.id = b.item \
             WHERE b.collection = 'T' ORDER BY b.source, b.handle",
        )
        .unwrap();
    stmt.query_map([], |row| {
        Ok(Row {
            source: row.get(0)?,
            handle: row.get(1)?,
            link_id: row.get(2)?,
            seq: row.get(3)?,
            status_base: row.get(4)?,
        })
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn items(dir: &Path) -> Vec<(String, String, i64)> {
    let conn = rusqlite::Connection::open(dir.join("pimdir.db")).unwrap();
    let mut stmt = conn
        .prepare("SELECT collection, link_id, seq FROM items ORDER BY collection, link_id")
        .unwrap();
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn projected(
    store: &PimdirSourceStore,
    collection: &str,
) -> Vec<(String, Option<String>, PimdirStatus)> {
    store
        .load(
            &PimdirCollectionId(collection.into()),
            &PimdirLoadSelection::All,
        )
        .unwrap()
        .placements
        .into_iter()
        .map(|p| (p.handle.0.clone(), p.link_id.map(|l| l.0), p.status))
        .collect()
}

fn seq_in_s(store: &PimdirSourceStore) -> i64 {
    store.list_items("S", None, 10).unwrap()[0].seq
}

fn row(source: &str, handle: &str, link: &str, seq: i64, base: i64) -> Row {
    Row {
        source: source.into(),
        handle: handle.into(),
        link_id: link.into(),
        seq,
        status_base: base,
    }
}

fn seeded_state(dir: &Path, left: &PimdirSourceStore) {
    assert_eq!(target(dir), vec![row("right", "t1", "mid:L", 1, 1)]);
    assert_eq!(
        projected(left, "T"),
        vec![(
            "\u{1}mid:L".to_string(),
            Some("mid:L".to_string()),
            PimdirStatus::Created
        )],
        "the store offers left the item right binds"
    );
}

fn enqueue(dir: &Path, left: &PimdirSourceStore, copy: bool) {
    let seq = seq_in_s(left);
    let to = PimdirCollectionId("T".into());
    let action = match copy {
        true => PimdirAction::Copy { seq, to },
        false => PimdirAction::Move { seq, to },
    };
    let mut producer = PimdirProducer::open(dir, "test").unwrap();
    producer.enqueue("S", &action, None).unwrap();
}

fn mutation(copy: bool) -> PimdirMutation {
    let handle = PimdirHandle("s1".into());
    let target = PimdirCollectionId("T".into());
    match copy {
        true => PimdirMutation::Copy { handle, target },
        false => PimdirMutation::Move { handle, target },
    }
}

/// The rows a Copy or a Move leaves, drained or mutated alike.
fn joined(dir: &Path, left: &PimdirSourceStore, copy: bool) {
    assert_eq!(
        target(dir),
        vec![
            row("left", "\u{1}mid:L", "mid:L", 1, 0),
            row("right", "t1", "mid:L", 1, 1),
        ],
        "left binds right's item under the bare key"
    );
    assert_eq!(
        items(dir),
        vec![
            ("S".to_string(), "mid:L".to_string(), 1),
            ("T".into(), "mid:L".into(), 1)
        ]
    );
    assert_eq!(
        projected(left, "T"),
        vec![(
            "\u{1}mid:L".to_string(),
            Some("mid:L".to_string()),
            PimdirStatus::Created
        )]
    );
    let source = match copy {
        true => PimdirStatus::Clean,
        false => PimdirStatus::Tombstone,
    };
    assert_eq!(projected(left, "S")[0].2, source);
}

fn drained(copy: bool) {
    let dir = tempfile::tempdir().unwrap();
    let mut left = seeded(dir.path());
    seeded_state(dir.path(), &left);
    enqueue(dir.path(), &left, copy);

    let report = left.drain().unwrap();

    assert_eq!((report.applied, report.parked), (1, 0));
    joined(dir.path(), &left, copy);
}

fn mutated(copy: bool) {
    let dir = tempfile::tempdir().unwrap();
    let mut left = seeded(dir.path());
    seeded_state(dir.path(), &left);

    left.mutate("S", mutation(copy)).unwrap();

    joined(dir.path(), &left, copy);
}

/// A queued Copy into T, where another source binds the identity, joins
/// that source's item.
#[test]
fn a_drained_copy_joins_the_other_sources_item() {
    drained(true);
}

/// The same Copy through `store.mutate`.
#[test]
fn a_mutated_copy_joins_the_other_sources_item() {
    mutated(true);
}

/// A queued Move into T, where another source binds the identity, joins
/// that source's item.
#[test]
fn a_drained_move_joins_the_other_sources_item() {
    drained(false);
}

/// The same Move through `store.mutate`.
#[test]
fn a_mutated_move_joins_the_other_sources_item() {
    mutated(false);
}

/// A queued Copy run as left, which syncs no T, where right binds the
/// identity as a tombstone whose delete is not pushed: the create is
/// staged under right, which syncs T, so it revives right's own tombstone
/// under t1, staging no create (pimdir SYNC §7, vector 230, which the
/// vector drain cannot express, two sources binding there).
#[test]
fn a_drained_copy_revives_the_tombstone_of_the_source_syncing_its_target() {
    let dir = tempfile::tempdir().unwrap();
    let mut right = PimdirStore::open(dir.path()).unwrap().for_source("right");
    right.ensure_collection("S", "message/rfc822").unwrap();
    right.ensure_collection("T", "message/rfc822").unwrap();
    right
        .write(vec![
            PimdirWriteOp::StoreObject {
                object: PimdirObject {
                    hash: hash::hash(BODY),
                    size: BODY.len(),
                },
                body: Some(BODY.to_vec()),
            },
            PimdirWriteOp::UpsertPlacement(placement("T", "t1")),
        ])
        .unwrap();
    right
        .mutate(
            "T",
            PimdirMutation::Remove {
                handle: PimdirHandle("t1".into()),
                if_match: None,
                resolves: None,
            },
        )
        .unwrap();
    let mut left = PimdirStore::open(dir.path()).unwrap().for_source("left");
    left.write(vec![PimdirWriteOp::UpsertPlacement(placement("S", "s1"))])
        .unwrap();
    enqueue(dir.path(), &left, true);

    let report = left.drain().unwrap();

    assert_eq!((report.applied, report.parked), (1, 0));
    assert_eq!(target(dir.path()), vec![row("right", "t1", "mid:L", 1, 1)]);
    assert_eq!(
        projected(&right, "T"),
        vec![(
            "t1".to_string(),
            Some("mid:L".to_string()),
            PimdirStatus::Clean
        )],
        "the tombstone revived, its delete withdrawn"
    );
}

/// A queued Copy run as left, which syncs no T, drained by zulu, which
/// syncs T beside alpha: the create is staged under zulu, the owner's own
/// among the sources syncing T, as an add there runs (pimdir STORAGE
/// §15.2, SYNC §7).
#[test]
fn a_drained_copy_stages_its_create_under_the_owners_own_source() {
    let dir = tempfile::tempdir().unwrap();
    let checkpoint = || PimdirWriteOp::SetCheckpoint {
        collection: PimdirCollectionId("T".into()),
        checkpoint: PimdirCheckpoint(b"cp".to_vec()),
    };
    let mut alpha = PimdirStore::open(dir.path()).unwrap().for_source("alpha");
    alpha.ensure_collection("S", "message/rfc822").unwrap();
    alpha.ensure_collection("T", "message/rfc822").unwrap();
    alpha.write(vec![checkpoint()]).unwrap();
    let mut left = PimdirStore::open(dir.path()).unwrap().for_source("left");
    left.write(vec![
        PimdirWriteOp::StoreObject {
            object: PimdirObject {
                hash: hash::hash(BODY),
                size: BODY.len(),
            },
            body: Some(BODY.to_vec()),
        },
        PimdirWriteOp::UpsertPlacement(placement("S", "s1")),
    ])
    .unwrap();
    enqueue(dir.path(), &left, true);
    let mut zulu = PimdirStore::open(dir.path()).unwrap().for_source("zulu");
    zulu.write(vec![checkpoint()]).unwrap();

    let report = zulu.drain().unwrap();

    assert_eq!((report.applied, report.parked), (1, 0));
    assert_eq!(
        target(dir.path()),
        vec![row("zulu", "\u{1}mid:L", "mid:L", 1, 0)]
    );
}
