//! A store never collects itself (spec §5, §14).
//!
//! Reclamation lives in a collector rather than in the write path, so
//! the pattern §14 invites works: stream a body straight to its sharded
//! path, index it, attach it in a later batch. A write that swept its own
//! refcount-zero objects would destroy the body the second batch was
//! going to reference, silently and bytes included.

#![cfg(feature = "client")]

use std::fs;

use io_pimdir::{
    change::{PimdirDropReason, PimdirWriteOp},
    collection::PimdirCollectionId,
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
};
use io_pimdir::{
    client::PimdirStore,
    sql,
    summary::{PimdirSummary, mail::PimdirMailSummary},
};

fn inbox() -> PimdirCollectionId {
    PimdirCollectionId("INBOX".into())
}

fn placement(handle: &str, link: &str, hash: &str) -> PimdirPlacement {
    PimdirPlacement {
        sort_key: Default::default(),
        collection: inbox(),
        handle: PimdirHandle(handle.into()),
        link_id: Some(PimdirLinkId(link.into())),
        object: Some(PimdirHash(hash.into())),
        level: PimdirLevel::Full,
        summary: None,
        flags: PimdirFlags::default(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags: PimdirFlags::default(),
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

fn blob_exists(dir: &std::path::Path, hash: &str) -> bool {
    dir.join("objects").join(&hash[0..2]).join(hash).exists()
}

/// The pattern the collector exists for: bodies stored in one batch,
/// attached in the next.
#[test]
fn a_body_stored_without_a_placement_survives_until_it_is_attached() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("remote");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();

    // batch one indexes two bodies and attaches neither, which is what a
    // consumer streaming bodies ahead of their metadata does
    store
        .write(vec![
            store_object(
                "cafebabecafebabecafebabecafebabecafebabecafebabecafe",
                b"first",
            ),
            store_object(
                "beefbeefbeefbeefbeefbeefbeefbeefbeefbeefbeefbeefbeef",
                b"second",
            ),
        ])
        .unwrap();
    assert!(blob_exists(
        dir.path(),
        "cafebabecafebabecafebabecafebabecafebabecafebabecafe"
    ));
    assert!(blob_exists(
        dir.path(),
        "beefbeefbeefbeefbeefbeefbeefbeefbeefbeefbeefbeefbeef"
    ));

    // batch two attaches one of them, both having survived
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(placement(
            "1",
            "mid:a",
            "cafebabecafebabecafebabecafebabecafebabecafebabecafe",
        ))])
        .unwrap();
    assert_eq!(
        store.list_items("INBOX", None, 10).unwrap()[0].object,
        Some(PimdirHash(
            "cafebabecafebabecafebabecafebabecafebabecafebabecafe".into()
        ))
    );

    // the one nothing attached is what a collection is for
    let collected = store.collect_garbage().unwrap();
    assert_eq!((collected.objects, collected.blobs), (1, 1));
    assert_eq!(collected.bytes, 6);
    assert!(
        blob_exists(
            dir.path(),
            "cafebabecafebabecafebabecafebabecafebabecafebabecafe"
        ),
        "still referenced"
    );
    assert!(
        !blob_exists(
            dir.path(),
            "beefbeefbeefbeefbeefbeefbeefbeefbeefbeefbeefbeefbeef"
        ),
        "referenced by nothing"
    );
}

/// The other half of what a collector takes: a file no row references,
/// which a crash between the blob write and the commit leaves behind.
#[test]
fn an_orphan_blob_is_collected_with_the_unreferenced_rows() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("remote");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    store
        .write(vec![
            store_object(
                "cafebabecafebabecafebabecafebabecafebabecafebabecafe",
                b"body",
            ),
            PimdirWriteOp::UpsertPlacement(placement(
                "1",
                "mid:a",
                "cafebabecafebabecafebabecafebabecafebabecafebabecafe",
            )),
        ])
        .unwrap();

    // a body on disk that no batch ever indexed
    let shard = dir.path().join("objects/de");
    fs::create_dir_all(&shard).unwrap();
    fs::write(
        shard.join("deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdead"),
        b"crashed",
    )
    .unwrap();

    // a half-written body no writer holds any more: under the collector's
    // locks it is what a crash left, taken and counted apart
    fs::write(dir.path().join("objects/.tmp-1-1"), b"in flight").unwrap();

    let collected = store.collect_garbage().unwrap();
    assert_eq!((collected.objects, collected.blobs), (0, 1));
    assert_eq!((collected.temporaries, collected.bytes), (1, 7 + 9));
    assert!(
        !shard
            .join("deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdead")
            .exists(),
        "the orphan is taken"
    );
    assert!(
        blob_exists(
            dir.path(),
            "cafebabecafebabecafebabecafebabecafebabecafebabecafe"
        ),
        "the live body is not"
    );
    assert!(!dir.path().join("objects/.tmp-1-1").exists());
}

/// A purge that removed rows gives the pages they freed back to the
/// filesystem, so pimdir.db shrinks once checkpointed (STORAGE §4.1): the
/// store is created with `auto_vacuum = INCREMENTAL`, which frees nothing
/// until an owner asks.
#[test]
fn a_large_purge_gives_its_pages_back_to_the_filesystem() {
    const ITEMS: usize = 1_500;

    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("remote");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    let size = || {
        let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .unwrap();
        fs::metadata(dir.path().join("pimdir.db")).unwrap().len()
    };

    let listed = (0..ITEMS).map(|n| {
        let summary = PimdirMailSummary {
            message_id: Some(format!("{n}@x")),
            in_reply_to: Vec::new(),
            subject: "subject ".repeat(64),
            sender: None,
            sender_name: None,
            date: None,
            size: None,
            attachment: None,
            invitation: None,
            from: Vec::new(),
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
        };
        PimdirWriteOp::UpsertPlacement(PimdirPlacement {
            object: None,
            level: PimdirLevel::Meta,
            summary: Some(PimdirSummary::Mail(summary)),
            base: Some(PimdirBase {
                flags: PimdirFlags::default(),
                revision: None,
                object: None,
            }),
            ..placement(&n.to_string(), &format!("mid:{n}@x"), "")
        })
    });
    store.write(listed.collect()).unwrap();
    let filled = size();

    let dropped = (0..ITEMS).map(|n| PimdirWriteOp::DropPlacement {
        collection: inbox(),
        handle: PimdirHandle(n.to_string()),
        reason: PimdirDropReason::Deleted,
    });
    store.write(dropped.collect()).unwrap();
    let purged = store
        .purge_retained_before("2100-01-01T00:00:00.000Z")
        .unwrap();
    assert_eq!(purged.items, ITEMS);
    store.collect_garbage().unwrap();

    let purged = size();
    assert!(
        purged < filled / 2,
        "pimdir.db went from {filled} to {purged} bytes"
    );
}

/// A purge retires rows and releases what they pinned; the bytes are the
/// collector's to report.
#[test]
fn a_purge_retires_rows_and_the_collector_reclaims_the_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("remote");
    store
        .write(vec![
            store_object(
                "cafebabecafebabecafebabecafebabecafebabecafebabecafe",
                b"body",
            ),
            PimdirWriteOp::UpsertPlacement(placement(
                "1",
                "mid:a",
                "cafebabecafebabecafebabecafebabecafebabecafebabecafe",
            )),
        ])
        .unwrap();
    store
        .write(vec![PimdirWriteOp::DropPlacement {
            collection: inbox(),
            handle: PimdirHandle("1".into()),
            reason: PimdirDropReason::Deleted,
        }])
        .unwrap();

    let purged = store
        .purge_retained_before("2100-01-01T00:00:00.000Z")
        .unwrap();
    assert_eq!(purged.items, 1);
    assert!(
        blob_exists(
            dir.path(),
            "cafebabecafebabecafebabecafebabecafebabecafebabecafe"
        ),
        "released, not reclaimed"
    );

    let collected = store.collect_garbage().unwrap();
    assert_eq!(collected.bytes, 4);
    assert!(!blob_exists(
        dir.path(),
        "cafebabecafebabecafebabecafebabecafebabecafebabecafe"
    ));
}

/// The repair `check --fix` runs: a drifted count settled from the
/// pointers that justify it, and the one dangling row it can clear.
#[test]
fn a_repair_recomputes_a_drifted_refcount_and_clears_a_dangling_binding() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("remote");
    store
        .write(vec![
            store_object(
                "cafebabecafebabecafebabecafebabecafebabecafebabecafe",
                b"body",
            ),
            PimdirWriteOp::UpsertPlacement(placement(
                "1",
                "mid:a",
                "cafebabecafebabecafebabecafebabecafebabecafebabecafe",
            )),
        ])
        .unwrap();

    // drift the count and dangle a binding the way a foreign writer
    // would: foreign keys are per connection and off by default, which is
    // how a store acquires a row the schema forbids
    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
    conn.execute("UPDATE objects SET refcount = 7", []).unwrap();
    conn.execute(
        "INSERT INTO bindings(item, collection, source, handle) \
         VALUES(424242, 'INBOX', 'remote', 'x')",
        [],
    )
    .unwrap();
    drop(conn);

    assert_eq!(store.recompute_refcounts().unwrap(), 1);
    assert_eq!(store.clear_dangling_bindings().unwrap(), 1);

    // settled, not swept: the item still references the body, so the
    // repair leaves it at one and the collector finds nothing
    assert_eq!(store.collect_garbage().unwrap().blobs, 0);
    assert!(blob_exists(
        dir.path(),
        "cafebabecafebabecafebabecafebabecafebabecafebabecafe"
    ));
    assert_eq!(
        store.recompute_refcounts().unwrap(),
        0,
        "nothing left to fix"
    );

    // and the statement is the canonical one, run against the real schema
    assert!(
        sql::CANONICAL
            .iter()
            .any(|(name, _)| *name == "RECOMPUTE_REFCOUNTS")
    );
}
