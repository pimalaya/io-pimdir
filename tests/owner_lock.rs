//! One owner, enforced (spec §8).
//!
//! The rule was stated and nothing kept it: SQLite's write lock serialises
//! statements without serialising operations, so two owners could each read a
//! consistent snapshot and then each act on it. An advisory lock on the store
//! directory is what makes "at most one owner" a fact rather than a convention
//! about who runs what, and the roles that are not owners — a reader, a
//! producer appending to the queue — go on working while it is held.

use std::{fs::File, fs::OpenOptions, path::Path};

use fs4::FileExt;
use io_pimdir::{PimdirError, PimdirProducer, PimdirStore, codec::PimdirAction};

/// Creates the store, then lets go of it.
fn create(dir: &Path) {
    PimdirStore::open(dir)
        .unwrap()
        .ensure_collection("INBOX", "message/rfc822")
        .unwrap();
}

fn owner_lock(dir: &Path) -> File {
    OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(dir.join("owner.lock"))
        .unwrap()
}

/// Holds the store the way another process would: through a file description
/// this crate's own registry knows nothing about.
fn own_elsewhere(dir: &Path) -> File {
    let file = owner_lock(dir);
    FileExt::try_lock(&file).unwrap();
    file
}

/// Whether the store is there for the taking, asked from outside.
fn unowned(dir: &Path) -> bool {
    FileExt::try_lock(&owner_lock(dir)).is_ok()
}

#[test]
fn a_second_owner_is_refused_rather_than_made_to_wait() {
    let dir = tempfile::tempdir().unwrap();
    create(dir.path());
    let held = own_elsewhere(dir.path());

    // Not `Busy`, which is the 30-second wait this replaces: a wait that
    // outlasts a sync's transaction is a stall with no signal.
    assert!(matches!(
        PimdirStore::open(dir.path()),
        Err(PimdirError::Owned(_))
    ));

    drop(held);
    assert!(PimdirStore::open(dir.path()).is_ok());
}

#[test]
fn a_reader_opens_while_another_process_owns_the_store() {
    let dir = tempfile::tempdir().unwrap();
    create(dir.path());
    let _held = own_elsewhere(dir.path());

    let reader = PimdirStore::open_read_only(dir.path()).unwrap();
    assert_eq!(reader.list_collections().unwrap().len(), 1);
}

#[test]
fn a_producer_appends_while_another_process_owns_the_store() {
    let dir = tempfile::tempdir().unwrap();
    create(dir.path());
    let _held = own_elsewhere(dir.path());

    // The queue exists for exactly this: a frontend stages a mutation while
    // the owner is mid-sync, and the owner drains it when it gets there.
    let id = PimdirProducer::open(dir.path(), "test")
        .unwrap()
        .enqueue(
            "INBOX",
            &PimdirAction::Remove { seq: 1 },
            None,
            "2026-01-01T00:00:00.000Z",
        )
        .unwrap();
    assert_eq!(id, 1);
}

#[test]
fn one_process_owning_a_store_twice_is_one_owner() {
    let dir = tempfile::tempdir().unwrap();
    create(dir.path());

    // A two-sided sync opens one handle per source, and a multi-account owner
    // one per account: the lock is the process's, taken once and shared.
    let left = PimdirStore::open(dir.path()).unwrap().for_source("left");
    let right = PimdirStore::open(dir.path()).unwrap().for_source("right");
    assert!(!unowned(dir.path()));

    drop(left);
    assert!(!unowned(dir.path()), "the store is still owned by `right`");

    drop(right);
    assert!(unowned(dir.path()), "the last handle released it");
}
