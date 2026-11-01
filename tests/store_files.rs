//! A store's files are its user's alone (pimdir STORAGE §18): its
//! directories `0700` and its files `0600`, the databases' `-wal` and
//! `-shm` included, a store another user can read or write refused by
//! every profile, and a lock file opened without following a link.
//!
//! Every test here clears the process's umask first, so a mode the crate
//! does not ask for shows as `0777` or `0666` rather than hiding behind
//! the usual `022`: the file holds no test that relies on another umask.

#![cfg(all(unix, feature = "client"))]

use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};

use io_pimdir::{
    client::{
        PimdirError, PimdirSourceStore, PimdirStore, producer::PimdirProducer, reader::PimdirReader,
    },
    codec::PimdirAction,
    placement::PimdirFlags,
};
use rustix::{fs::Mode, process::umask};

const MAIL: &[u8] = b"Message-ID: <modes@x>\r\nSubject: modes\r\n\r\nbody";

/// Clears the process's umask.
fn unmasked() {
    umask(Mode::empty());
}

/// The permission bits of what sits at `path`, not following a link.
fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
}

/// Sets the permission bits of `path`.
fn chmod(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

/// A store at `dir`, its INBOX declared and one message enqueued and
/// applied, then its body flipped and drained again, so objects/, a shard,
/// a blob, both lock files and quarantine/ all exist.
fn populated(dir: &Path) -> PimdirSourceStore {
    let mut store = PimdirStore::open(dir).unwrap().for_source("local");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();

    let mut producer = PimdirProducer::open(dir, "test").unwrap();
    let object = producer.stage_body(MAIL).unwrap();
    let add = PimdirAction::Add {
        link_id: None,
        flags: PimdirFlags::default(),
        object: Some(object.hash.clone()),
        supersedes: None,
    };
    producer.enqueue("INBOX", &add, Some(&object)).unwrap();
    drop(producer);
    assert_eq!(store.drain().unwrap().applied, 1);

    let mut producer = PimdirProducer::open(dir, "test").unwrap();
    let other = producer.stage_body(b"From: x\r\n\r\nother").unwrap();
    let add = PimdirAction::Add {
        link_id: None,
        flags: PimdirFlags::default(),
        object: Some(other.hash.clone()),
        supersedes: None,
    };
    producer.enqueue("INBOX", &add, Some(&other)).unwrap();
    drop(producer);
    let path = store.blobs().path(&other.hash).unwrap();
    let mut bytes = fs::read(&path).unwrap();
    bytes[0] ^= 1;
    fs::write(&path, &bytes).unwrap();
    store.drain().unwrap();
    assert_eq!(store.blobs().quarantined().unwrap().len(), 1);

    store
}

/// A store created by this crate holds directories `0700` and files
/// `0600` alone, whatever the umask: the store directory and every parent
/// it made, objects/, a shard and quarantine/; pimdir.db with its `-wal`
/// and `-shm`, both lock files, a blob and a quarantined body.
#[test]
fn a_created_store_holds_directories_0700_and_files_0600() {
    unmasked();
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("accounts/store");
    let store = populated(&dir);

    let blob = store.blobs().path(&store.blobs().hash(MAIL)).unwrap();
    let shard = blob.parent().unwrap().to_path_buf();
    let quarantined = store.blobs().quarantined().unwrap().remove(0);

    for path in [
        root.path().join("accounts"),
        dir.clone(),
        dir.join("objects"),
        shard,
        dir.join("quarantine"),
    ] {
        assert_eq!(mode(&path), 0o700, "{}", path.display());
    }
    for path in [
        dir.join("pimdir.db"),
        dir.join("pimdir.db-wal"),
        dir.join("pimdir.db-shm"),
        dir.join("owner.lock"),
        dir.join("objects.lock"),
        blob,
        quarantined,
    ] {
        assert_eq!(mode(&path), 0o600, "{}", path.display());
    }
}

/// index.db, its `-wal` and `-shm` and index.lock are `0600` too (SEARCH
/// §3, STORAGE §18).
#[cfg(feature = "search")]
#[test]
fn the_index_files_are_0600() {
    use io_pimdir::client::search::{PimdirIndex, PimdirIndexRefreshOptions};

    unmasked();
    let dir = tempfile::tempdir().unwrap();
    let _store = populated(dir.path());
    let mut index = PimdirIndex::open(dir.path()).unwrap();
    index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();

    for name in ["index.db", "index.db-wal", "index.db-shm", "index.lock"] {
        assert_eq!(mode(&dir.path().join(name)), 0o600, "{name}");
    }
}

/// The directory a store is created in is restricted to its user, whoever
/// made it: the store's creator decides its mode.
#[test]
fn a_store_created_in_an_open_directory_restricts_it() {
    unmasked();
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("store");
    fs::create_dir(&dir).unwrap();
    chmod(&dir, 0o755);

    drop(PimdirStore::open(&dir).unwrap());
    assert_eq!(mode(&dir), 0o700);
}

/// A store whose directory or pimdir.db another user can read or write is
/// refused by every profile as `Exposed`, naming it, before anything is
/// opened; restricted again, it opens.
#[test]
fn a_store_others_can_read_or_write_is_refused_by_every_profile() {
    unmasked();
    let dir = tempfile::tempdir().unwrap();
    drop(PimdirStore::open(dir.path()).unwrap());
    let db = dir.path().join("pimdir.db");

    for (path, open) in [
        (dir.path(), 0o755),
        (dir.path(), 0o770),
        (db.as_path(), 0o644),
        (db.as_path(), 0o602),
    ] {
        chmod(path, open);
        let refusals = [
            ("owner", PimdirStore::open(dir.path()).err()),
            ("open_current", PimdirStore::open_current(dir.path()).err()),
            ("reader", PimdirReader::open(dir.path()).err()),
            ("producer", PimdirProducer::open(dir.path(), "test").err()),
        ];
        for (profile, refused) in refusals {
            let refused = refused.unwrap_or_else(|| panic!("{profile} opened at {open:o}"));
            assert!(
                matches!(&refused, PimdirError::Exposed(named) if named == path),
                "{profile} at {open:o}: {refused:?}"
            );
            assert_eq!(refused.code(), "store-exposed");
        }
        chmod(path, if path == dir.path() { 0o700 } else { 0o600 });
    }

    drop(PimdirStore::open(dir.path()).unwrap());
}

/// A link where a lock file belongs fails the open that takes the lock,
/// naming it, and the file it points at is neither created nor locked
/// (STORAGE §18): the owner lock, the staging lock a producer and the
/// collector take.
#[test]
fn a_link_where_a_lock_belongs_is_never_followed() {
    unmasked();
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let store = PimdirStore::open(dir.path()).unwrap();
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    drop(store);

    let owner = dir.path().join("owner.lock");
    let target = elsewhere.path().join("owner");
    fs::remove_file(&owner).unwrap();
    symlink(&target, &owner).unwrap();
    let refused = PimdirStore::open(dir.path()).err();
    assert!(
        matches!(&refused, Some(PimdirError::Exposed(named)) if *named == owner),
        "{refused:?}"
    );
    assert!(!target.exists(), "the link's target is not created");
    fs::remove_file(&owner).unwrap();

    let objects = dir.path().join("objects.lock");
    let target = elsewhere.path().join("objects");
    let _ = fs::remove_file(&objects);
    symlink(&target, &objects).unwrap();
    let producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let refused = producer.stage_body(MAIL).err();
    assert!(refused.is_some(), "a producer stages under no link");
    drop(producer);
    let mut store = PimdirStore::open(dir.path()).unwrap();
    let refused = store.collect_garbage().err();
    assert!(
        matches!(&refused, Some(PimdirError::Exposed(named)) if *named == objects),
        "{refused:?}"
    );
    assert!(!target.exists(), "the link's target is not created");
}

/// A link where index.lock belongs fails the refresh, naming it (STORAGE
/// §18, SEARCH §3).
#[cfg(feature = "search")]
#[test]
fn a_link_where_the_index_lock_belongs_is_never_followed() {
    use io_pimdir::client::search::{PimdirIndex, PimdirIndexRefreshOptions};

    unmasked();
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    drop(PimdirStore::open(dir.path()).unwrap());
    let mut index = PimdirIndex::open(dir.path()).unwrap();

    let lock = dir.path().join("index.lock");
    let target = elsewhere.path().join("index");
    let _ = fs::remove_file(&lock);
    symlink(&target, &lock).unwrap();
    let refused = index.refresh(&PimdirIndexRefreshOptions::default()).err();
    assert!(
        matches!(&refused, Some(PimdirError::Exposed(named)) if *named == lock),
        "{refused:?}"
    );
    assert!(!target.exists(), "the link's target is not created");
}
