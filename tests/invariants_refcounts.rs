//! Reference counts after the owner's removals (STORAGE §5, §7, §14),
//! ported from pimdir's retired checks/invariants.sh: a body's count is
//! what its pointers add up to once a collection goes, and never falls
//! below zero.

#![cfg(feature = "client")]

mod invariants_common;

use io_pimdir::change::PimdirWriteOp;

use invariants_common::*;

const AUG: &str = "Sat, 1 Aug 2026 10:00:00 +0000";

/// L520: a collection whose only pin is an agreement point holds an
/// object, so its delete settles that pin.
#[test]
fn l520_a_collection_pinning_only_an_agreed_body_settles_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    let a = message("a@x", "A", AUG, Some("alice@x"));
    store
        .write(vec![
            body(&store, &a),
            PimdirWriteOp::UpsertPlacement(held(&store, "Archive", "1", &a)),
        ])
        .unwrap();
    let hash = store.hash(&a);
    let conn = peek(dir.path());
    conn.execute_batch(
        "UPDATE items SET object = NULL, level = 1 WHERE collection = 'Archive';
         UPDATE bindings SET base_object = NULL WHERE collection = 'Archive';",
    )
    .unwrap();
    store.recompute_refcounts().unwrap();
    assert_eq!(
        int(
            &conn,
            "SELECT count(*) FROM bindings WHERE shared_object IS NOT NULL"
        ),
        1,
        "the agreement point alone names the body"
    );
    assert_eq!(refcount(dir.path(), &hash), Some(1));

    assert!(store.delete_collection("Archive").unwrap());
    assert!(
        store.refcount_drift().unwrap().is_empty(),
        "the cascade's pin settled in the delete"
    );
    assert_eq!(refcount(dir.path(), &hash), Some(0));
}

/// L574: after a collection's delete cascades its items, their bodies are
/// at zero.
#[test]
fn l574_a_deleted_collection_leaves_its_bodies_at_zero() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    let a = message("a@x", "A", AUG, Some("alice@x"));
    let b = message("b@x", "B", AUG, Some("alice@x"));
    store
        .write(vec![
            body(&store, &a),
            body(&store, &b),
            PimdirWriteOp::UpsertPlacement(held(&store, "Archive", "1", &a)),
            PimdirWriteOp::UpsertPlacement(held(&store, "Archive", "2", &b)),
        ])
        .unwrap();
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(held(
            &store, "INBOX", "1", &b,
        ))])
        .unwrap();

    assert!(store.delete_collection("Archive").unwrap());
    assert!(store.refcount_drift().unwrap().is_empty());
    assert_eq!(refcount(dir.path(), &store.hash(&a)), Some(0));
    assert!(
        refcount(dir.path(), &store.hash(&b)).unwrap() > 0,
        "the body INBOX holds keeps its pins"
    );
    let collected = store.collect_garbage().unwrap();
    assert_eq!(collected.objects, 1);
    assert_eq!(refcount(dir.path(), &store.hash(&a)), None);
}

/// L2689: a release past the pins taken is refused, the count kept.
#[test]
fn l2689_a_count_never_falls_below_zero() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    let loose = b"loose".to_vec();
    store.write(vec![body(&store, &loose)]).unwrap();
    let hash = store.hash(&loose);
    let conn = peek(dir.path());
    assert_eq!(refcount(dir.path(), &hash), Some(0));

    let write = conn.execute(
        "UPDATE objects SET refcount = refcount - 1 WHERE hash = ?1",
        [&hash.0],
    );
    assert!(refused(write, "refcount >= 0"));
    assert_eq!(refcount(dir.path(), &hash), Some(0), "the count kept");
}

/// L2692: an over-release of a pinned body is refused whole.
#[test]
fn l2692_an_over_release_is_refused_whole() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    let a = message("a@x", "A", AUG, Some("alice@x"));
    store
        .write(vec![
            body(&store, &a),
            PimdirWriteOp::UpsertPlacement(held(&store, "INBOX", "1", &a)),
        ])
        .unwrap();
    let hash = store.hash(&a);
    let count = refcount(dir.path(), &hash).unwrap();
    let conn = peek(dir.path());

    let write = conn.execute(
        "UPDATE objects SET refcount = refcount - ?2 WHERE hash = ?1",
        rusqlite::params![hash.0, count + 1],
    );
    assert!(refused(write, "refcount >= 0"));
    assert_eq!(refcount(dir.path(), &hash), Some(count), "the count kept");
}
