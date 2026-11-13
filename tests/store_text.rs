//! The store rules draft 10 put into text (pimdir STORAGE §3, §7, §8,
//! §11.3, §18; SYNC §7): the owner's local/, a lock file that is never
//! replaced, a backup and a restore, the deep check, and a delivered
//! create that no eviction takes and only a person gives up.

#![cfg(all(feature = "client", unix))]

use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use io_pimdir::{
    change::PimdirWriteOp,
    client::{
        PimdirError, PimdirRunError, PimdirSourceStore, PimdirStore,
        producer::{PimdirActionStatus, PimdirProducer},
        reader::PimdirReader,
    },
    codec::PimdirAction,
    collection::PimdirCollectionId,
    load::PimdirLoadSelection,
    mutate::{PimdirMutateError, PimdirMutation},
    object::PimdirHash,
    placement::{PimdirFlags, PimdirHandle, PimdirLinkId},
    sync::PimdirSyncEvent,
};

/// A message `id` dated in August 2026.
fn mail(id: &str) -> Vec<u8> {
    format!(
        "Message-ID: <{id}>\r\nFrom: Alice <alice@example.org>\r\nSubject: {id}\r\n\
         Date: Sat, 1 Aug 2026 10:00:00 +0000\r\n\r\nbody of {id}\r\n"
    )
    .into_bytes()
}

fn add(hash: PimdirHash) -> PimdirAction {
    PimdirAction::Add {
        link_id: None,
        flags: PimdirFlags::from_iter(["\\Seen"]),
        object: Some(hash),
        supersedes: None,
    }
}

/// A store whose INBOX holds the messages `ids`, each a create queued
/// and applied, pending for the source `local`.
fn populated(dir: &Path, ids: &[&str]) -> PimdirSourceStore {
    let mut store = PimdirStore::open(dir).unwrap().for_source("local");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    let mut producer = PimdirProducer::open(dir, "test").unwrap();
    for id in ids {
        producer.enqueue_with_body("INBOX", &mail(id), add).unwrap();
    }
    drop(producer);
    assert_eq!(store.drain().unwrap().applied, ids.len());
    store
}

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
}

/// Every regular file under `dir`, relative to it, with its bytes.
fn files(dir: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            match fs::symlink_metadata(&path).unwrap().is_dir() {
                true => walk(root, &path, out),
                false => out.push((
                    path.strip_prefix(root).unwrap().display().to_string(),
                    fs::read(&path).unwrap(),
                )),
            }
        }
    }

    let mut out = Vec::new();
    if dir.exists() {
        walk(dir, dir, &mut out);
    }
    out.sort();
    out
}

/// local/ is the owner's own (STORAGE §3, §18): created `0700` when the
/// owner first asks for it, never walked by the collector, refused by
/// the owner's open when another user can reach it, and ignored by a
/// reader and a producer.
#[test]
fn the_local_directory_is_the_owners_alone() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = populated(dir.path(), &["a@x"]);
    assert!(!dir.path().join("local").exists(), "created on demand");

    let local = store.local_dir().unwrap();
    assert_eq!(local, dir.path().join("local"));
    assert_eq!(mode(&local), 0o700);
    assert_eq!(store.local_dir().unwrap(), local, "found again");

    // what an owner keeps there is its own, whatever its name
    fs::write(local.join(".mark"), b"work under way").unwrap();
    fs::create_dir(local.join("ab")).unwrap();
    fs::write(local.join("ab/abnobody"), b"no body").unwrap();
    let report = store.collect_garbage().unwrap();
    assert_eq!((report.blobs, report.temporaries), (0, 0));
    assert!(
        report.strays.is_empty(),
        "never walked: {:?}",
        report.strays
    );
    assert_eq!(fs::read(local.join(".mark")).unwrap(), b"work under way");
    assert!(local.join("ab/abnobody").exists());
    drop(store);

    fs::set_permissions(&local, fs::Permissions::from_mode(0o755)).unwrap();
    let Err(PimdirError::Exposed(path)) = PimdirStore::open(dir.path()) else {
        panic!("an owner opened a store whose local/ others can read");
    };
    assert_eq!(path, local);
    assert!(matches!(
        PimdirStore::open_current(dir.path()),
        Err(PimdirError::Exposed(_))
    ));
    assert!(
        PimdirReader::open(dir.path()).is_ok(),
        "a reader ignores it"
    );
    assert!(PimdirProducer::open(dir.path(), "test").is_ok());

    // a link where the directory belongs is no directory of the owner's
    fs::set_permissions(&local, fs::Permissions::from_mode(0o700)).unwrap();
    let store = PimdirStore::open(dir.path()).unwrap();
    fs::remove_dir_all(&local).unwrap();
    std::os::unix::fs::symlink(dir.path(), &local).unwrap();
    assert!(matches!(store.local_dir(), Err(PimdirError::Exposed(_))));
}

/// A lock file is never removed nor replaced (STORAGE §8): an owner
/// whose owner.lock is no longer the file at its path fails its next
/// write before anything is written, and every write after it, since
/// another process may own the store through the new file.
#[test]
fn an_owner_whose_lock_file_was_replaced_writes_nothing_more() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = populated(dir.path(), &["a@x"]);
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let queued = producer
        .enqueue_with_body("INBOX", &mail("b@x"), add)
        .unwrap()
        .handle;
    drop(producer);
    let seq = store.list_items("INBOX", None, 10).unwrap()[0].seq;

    let lock = dir.path().join("owner.lock");
    fs::remove_file(&lock).unwrap();
    fs::File::create(&lock).unwrap();

    // another process takes the new file's lock while this owner lives
    let other = fs::File::open(&lock).unwrap();
    assert!(
        rustix::fs::flock(&other, rustix::fs::FlockOperation::NonBlockingLockExclusive).is_ok(),
        "the old lock excludes nobody at the path"
    );

    let refused = store
        .ensure_collection("Sent", "message/rfc822")
        .unwrap_err();
    assert_eq!(refused.code(), "lock-replaced");
    assert!(matches!(refused, PimdirError::LockReplaced(path) if path == lock));

    let body = b"never written";
    let object = io_pimdir::object::PimdirObject {
        hash: store.hash(body),
        size: body.len(),
    };
    let write = store.write(vec![PimdirWriteOp::StoreObject {
        object: object.clone(),
        body: Some(body.to_vec()),
    }]);
    assert!(matches!(write, Err(PimdirError::LockReplaced(_))));
    assert!(!store.blobs().path(&object.hash).unwrap().exists());

    assert!(matches!(store.drain(), Err(PimdirError::LockReplaced(_))));
    let handle = store
        .item_bindings(
            "INBOX",
            &store.get_item("INBOX", seq).unwrap().unwrap().link_id.0,
        )
        .unwrap()
        .into_values()
        .next()
        .unwrap()
        .handle;
    let mutated = store.mutate(
        "INBOX",
        PimdirMutation::SetFlags {
            handle,
            flags: PimdirFlags::default(),
            base: None,
        },
    );
    assert!(matches!(
        mutated,
        Err(PimdirRunError::Store(PimdirError::LockReplaced(_)))
    ));
    assert!(matches!(
        store.drop_action(&queued),
        Err(PimdirError::LockReplaced(_))
    ));
    assert!(matches!(
        store.fail_action(queued.id, Some("x")),
        Err(PimdirError::LockReplaced(_))
    ));

    // nothing was written: the store reads as it stood
    let reader = PimdirReader::open(dir.path()).unwrap();
    assert_eq!(reader.list_collections().unwrap().len(), 1);
    assert!(matches!(
        reader.action_status(&queued).unwrap(),
        PimdirActionStatus::Pending { attempts: 0, .. }
    ));
    let item = reader.get_item("INBOX", seq).unwrap().unwrap();
    assert!(item.flags.contains("\\Seen"));

    // the file put back changes nothing: the owner gave the store up
    drop(other);
    assert!(matches!(
        store.ensure_collection("Sent", "message/rfc822"),
        Err(PimdirError::LockReplaced(_))
    ));
}

/// A producer checks the staging lock it holds the same way (STORAGE
/// §8): one whose objects.lock was replaced under its staging window
/// enqueues nothing, a collector locking the new file not seeing it.
#[test]
fn a_producer_whose_staging_lock_was_replaced_enqueues_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = populated(dir.path(), &["a@x"]);
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let object = producer.stage_body(&mail("b@x")).unwrap();

    let lock = dir.path().join("objects.lock");
    fs::remove_file(&lock).unwrap();
    fs::File::create(&lock).unwrap();

    let refused = producer
        .enqueue("INBOX", &add(object.hash.clone()), Some(&object))
        .unwrap_err();
    assert_eq!(refused.code(), "lock-replaced");
    assert!(store.list_pending_actions().unwrap().is_empty());
    assert!(!store.indexed_hashes().unwrap().contains(&object.hash.0));
}

/// A backup taken while a producer enqueues and the owner drains is
/// consistent (STORAGE §8): restored, its database passes the deep
/// check and every body it names, a queued row's included, is there.
/// The copy is its user's alone, and holds neither lock nor index.
#[test]
fn a_backup_taken_under_a_running_producer_is_consistent() {
    let dir = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let mut store = populated(dir.path(), &["a@x", "b@x", "c@x"]);
    fs::write(store.local_dir().unwrap().join("state"), b"owner state").unwrap();
    // a writer's temporary and a quarantined file
    let shard = fs::read_dir(dir.path().join("objects"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(shard.join(".crashed.tmp"), b"half").unwrap();
    fs::create_dir(dir.path().join("quarantine")).unwrap();
    fs::write(dir.path().join("quarantine/failed-1"), b"set aside").unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let enqueuing = {
        let (stop, dir) = (Arc::clone(&stop), dir.path().to_path_buf());
        thread::spawn(move || {
            let mut producer = PimdirProducer::open(&dir, "busy").unwrap();
            let mut queued = 0;
            while !stop.load(Ordering::Acquire) {
                let body = mail(&format!("busy-{queued}@x"));
                producer.enqueue_with_body("INBOX", &body, add).unwrap();
                queued += 1;
                // NOTE: a frontend's pace, so the owner gets its turn at
                // the write lock.
                thread::sleep(Duration::from_millis(2));
            }
            queued
        })
    };

    let reader = PimdirReader::open(dir.path()).unwrap();
    let mut backups = Vec::new();
    for round in 0..4 {
        let target = scratch.path().join(format!("backup-{round}"));
        let copied = reader.backup(&target).unwrap();
        assert!(copied.objects >= 3, "{copied:?}");
        assert_eq!((copied.quarantined, copied.local, copied.kept), (1, 1, 0));
        assert!(copied.skipped.is_empty(), "{:?}", copied.skipped);
        backups.push(target);
        assert!(store.drain().unwrap().applied > 0);
        // NOTE: refused while the producer stages, which is its right.
        let _ = store.collect_garbage();
    }
    stop.store(true, Ordering::Release);
    assert!(enqueuing.join().unwrap() > 0);

    assert!(matches!(
        reader.backup(&backups[0]),
        Err(PimdirError::Occupied(_))
    ));

    for (round, backup) in backups.iter().enumerate() {
        assert_eq!(mode(backup), 0o700);
        assert_eq!(mode(&backup.join("objects")), 0o700);
        assert_eq!(mode(&backup.join("local")), 0o700);
        let journal: String = rusqlite::Connection::open(backup.join("pimdir.db"))
            .unwrap()
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(journal, "delete", "one file, whatever the store's mode");
        for (name, _) in files(backup) {
            assert_eq!(mode(&backup.join(&name)), 0o600, "{name}");
            assert!(!name.ends_with(".lock") && !name.contains(".tmp"), "{name}");
            assert!(!name.starts_with("index"), "{name}");
        }
        assert_eq!(
            fs::read(backup.join("local/state")).unwrap(),
            b"owner state"
        );
        assert_eq!(
            fs::read(backup.join("quarantine/failed-1")).unwrap(),
            b"set aside"
        );

        let restored = scratch.path().join(format!("restored-{round}"));
        let report = PimdirStore::restore(&restored, backup).unwrap();
        assert_eq!(report.replaced, None);
        let copy = PimdirReader::open(&restored).unwrap();
        let deep = copy.check_deep().unwrap();
        assert!(deep.is_sound(), "backup {round}: {deep:?}");
        assert!(deep.objects >= 3);
        assert!(copy.dangling().unwrap().is_empty());
        assert!(copy.refcount_drift().unwrap().is_empty());
        for queued in copy.list_pending_actions().unwrap() {
            let hash = queued.action.object_hash().expect("an add names its body");
            assert!(copy.blobs().get(hash).unwrap().is_some(), "queued body");
        }
    }
}

/// A restore replaces the store by the backup under a new incarnation
/// (STORAGE §4.5, §8): the store keeps its id, the incarnation rises
/// above the backup's and the replaced store's by a stride no two
/// restores share, local/ is the backup's, the lock files stay the files
/// they were, and what the store held since the backup is gone.
#[test]
fn a_restore_answers_a_new_incarnation() {
    let dir = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let backup = scratch.path().join("backup");

    let mut store = populated(dir.path(), &["a@x", "b@x"]);
    fs::write(store.local_dir().unwrap().join("state"), b"at the backup").unwrap();
    let identity = store.change_cursor().unwrap();
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let queued = producer
        .enqueue_with_body("INBOX", &mail("queued@x"), add)
        .unwrap()
        .handle;
    store.backup(&backup).unwrap();

    // the store moves on after the backup
    producer
        .enqueue_with_body("INBOX", &mail("later@x"), add)
        .unwrap();
    drop(producer);
    assert_eq!(store.drain().unwrap().applied, 2);
    fs::write(store.local_dir().unwrap().join("state"), b"since").unwrap();
    fs::write(store.local_dir().unwrap().join("newer"), b"since").unwrap();
    store.raise_incarnation(None).unwrap();
    let replaced = store.change_cursor().unwrap().incarnation;
    let lock_inode = fs::metadata(dir.path().join("owner.lock")).unwrap().ino();

    // a handle of this process still open: nothing is replaced under it
    assert!(matches!(
        PimdirStore::restore(dir.path(), &backup),
        Err(PimdirError::Owned(_))
    ));
    assert_eq!(store.count_items("INBOX").unwrap(), 4);
    drop(store);

    let first = PimdirStore::restore(dir.path(), &backup).unwrap();
    assert_eq!(first.replaced, Some(replaced));
    assert!(first.incarnation > replaced && first.incarnation > identity.incarnation);
    assert_eq!(first.copied.local, 1);

    let store = PimdirStore::open(dir.path()).unwrap();
    let cursor = store.change_cursor().unwrap();
    assert_eq!(cursor.store_id, identity.store_id, "the store keeps its id");
    assert_eq!(cursor.incarnation, first.incarnation);
    assert_eq!(store.count_items("INBOX").unwrap(), 2, "as backed up");
    assert_eq!(store.list_pending_actions().unwrap().len(), 1);
    assert_eq!(
        store.action_status(&queued).unwrap(),
        PimdirActionStatus::Reset,
        "a handle of the store before the restore follows nothing"
    );
    let local = store.local_dir().unwrap();
    assert_eq!(fs::read(local.join("state")).unwrap(), b"at the backup");
    assert!(!local.join("newer").exists(), "local/ is the backup's");
    assert_eq!(files(&local).len(), 1, "nothing of the restore's left");
    assert_eq!(
        fs::metadata(dir.path().join("owner.lock")).unwrap().ino(),
        lock_inode,
        "a lock file is never replaced"
    );
    let journal: String = rusqlite::Connection::open(dir.path().join("pimdir.db"))
        .unwrap()
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(journal, "wal", "the journal mode of the store replaced");
    assert!(store.check_deep().unwrap().is_sound());
    drop(store);

    // the same backup restored again answers another incarnation
    let second = PimdirStore::restore(dir.path(), &backup).unwrap();
    assert_eq!(second.replaced, Some(first.incarnation));
    assert!(second.incarnation > first.incarnation);
    let elsewhere = scratch.path().join("elsewhere");
    let third = PimdirStore::restore(&elsewhere, &backup).unwrap();
    assert_eq!(third.replaced, None);
    assert_ne!(third.incarnation, identity.incarnation);
    assert_ne!(third.incarnation, second.incarnation);

    // no backup, no store touched
    assert!(matches!(
        PimdirStore::restore(dir.path(), scratch.path().join("absent")),
        Err(PimdirError::Uncreated)
    ));
    assert_eq!(
        PimdirReader::open(dir.path())
            .unwrap()
            .change_cursor()
            .unwrap()
            .incarnation,
        second.incarnation
    );
}

/// The deep check hashes every body an object row names (STORAGE §7): a
/// flipped byte is reported, and a missing body, and nothing is moved.
#[test]
fn a_flipped_byte_is_reported_by_the_deep_check() {
    let dir = tempfile::tempdir().unwrap();
    let store = populated(dir.path(), &["a@x", "b@x", "c@x"]);
    let reader = PimdirReader::open(dir.path()).unwrap();
    let sound = reader.check_deep().unwrap();
    assert!(sound.is_sound(), "{sound:?}");
    assert_eq!(sound.objects, 3);

    let mut hashes = store.indexed_hashes().unwrap().into_iter();
    let flipped = PimdirHash(hashes.next().unwrap());
    let removed = PimdirHash(hashes.next().unwrap());
    let path = store.blobs().path(&flipped).unwrap();
    let mut bytes = fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(&path, &bytes).unwrap();
    fs::remove_file(store.blobs().path(&removed).unwrap()).unwrap();

    let found = reader.check_deep().unwrap();
    assert!(!found.is_sound());
    assert_eq!(found.failing, vec![flipped]);
    assert_eq!(found.missing, vec![removed]);
    assert!(found.integrity.is_empty() && found.foreign_keys.is_empty());
    assert!(path.exists(), "reported, never moved");
    assert!(store.blobs().quarantined().unwrap().is_empty());
}

/// The provisional handle a create of `link` takes.
fn provisional(link: &str) -> PimdirHandle {
    PimdirLinkId(link.into()).provisional()
}

/// A store holding one create of August 2026 the source accepted with
/// no handle: a delivered create, based under its provisional handle.
fn delivered(dir: &Path) -> (PimdirSourceStore, PimdirHandle) {
    let mut store = populated(dir, &["a@x"]);
    let handle = provisional("a@x");
    store
        .write(vec![PimdirWriteOp::DeliverPlacement {
            collection: PimdirCollectionId("INBOX".into()),
            handle: handle.clone(),
        }])
        .unwrap();
    let loaded = store
        .load(
            &PimdirCollectionId("INBOX".into()),
            &PimdirLoadSelection::All,
        )
        .unwrap();
    assert!(loaded.placements[0].delivered(), "{:?}", loaded.placements);
    (store, handle)
}

/// The one item row of the store: `deleted`, `retained_by`, its body and
/// its system flags.
fn item_row(dir: &Path) -> (i64, Option<String>, Option<String>, i64) {
    rusqlite::Connection::open(dir.join("pimdir.db"))
        .unwrap()
        .query_row(
            "SELECT deleted, retained_by, object_hash, marks FROM items",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap()
}

/// How many bindings the store holds.
fn bindings(dir: &Path) -> i64 {
    rusqlite::Connection::open(dir.join("pimdir.db"))
        .unwrap()
        .query_row("SELECT count(*) FROM bindings", [], |r| r.get(0))
        .unwrap()
}

/// No eviction takes a delivered create (STORAGE §11.3): what was staged
/// on it is owed to the page that lands it, and nothing relists a member
/// no listing has named.
#[test]
fn an_eviction_keeps_a_delivered_create() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, _) = delivered(dir.path());

    let report = store.evict_before("INBOX", "2026-10-01T00:00:00Z").unwrap();
    assert!(report.seqs.is_empty(), "{:?}", report.seqs);
    assert_eq!(store.count_items("INBOX").unwrap(), 1);
}

/// A delivered create is given up on a person's request and by nothing
/// else (SYNC §7): the `Abandon` drops its binding, the item is retained
/// with its body and its flags, and the mutation reports the provisional
/// handle. A tombstoned one goes the same way, its held remove with the
/// binding; a pending create is refused.
#[test]
fn a_delivered_create_is_abandoned_on_request_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, handle) = delivered(dir.path());
    let object = store.list_items("INBOX", None, 10).unwrap()[0]
        .object
        .clone();

    let report = store
        .mutate(
            "INBOX",
            PimdirMutation::Abandon {
                handle: handle.clone(),
            },
        )
        .unwrap();
    assert_eq!(report.events, vec![PimdirSyncEvent::Abandoned(handle)]);
    assert_eq!(store.count_items("INBOX").unwrap(), 0);
    assert_eq!(store.count_retained("INBOX").unwrap(), 1, "retained");
    let (deleted, retained_by, body, marks) = item_row(dir.path());
    assert_eq!(deleted, 1);
    assert_eq!(retained_by.as_deref(), Some("local"));
    assert_eq!(body.map(PimdirHash), object, "its body kept");
    assert_eq!(marks & 1, 1, "the flag staged on it kept");
    assert_eq!(bindings(dir.path()), 0, "no binding left");

    // a tombstoned delivered create: the remove held on it is given up
    let other = tempfile::tempdir().unwrap();
    let (mut store, handle) = delivered(other.path());
    store
        .mutate(
            "INBOX",
            PimdirMutation::Remove {
                handle: handle.clone(),
                if_match: None,
                resolves: None,
            },
        )
        .unwrap();
    assert_eq!(bindings(other.path()), 1, "the remove held on it");
    let report = store
        .mutate(
            "INBOX",
            PimdirMutation::Abandon {
                handle: handle.clone(),
            },
        )
        .unwrap();
    assert_eq!(report.events, vec![PimdirSyncEvent::Abandoned(handle)]);
    assert_eq!(bindings(other.path()), 0, "given up with the binding");
    assert_eq!(store.count_retained("INBOX").unwrap(), 1);

    // a pending create is withdrawn by a remove, never abandoned
    let pending = tempfile::tempdir().unwrap();
    let mut store = populated(pending.path(), &["a@x"]);
    let refused = store.mutate(
        "INBOX",
        PimdirMutation::Abandon {
            handle: provisional("a@x"),
        },
    );
    let Err(PimdirRunError::Coroutine(err)) = refused else {
        panic!("a pending create was abandoned");
    };
    assert!(matches!(err, PimdirMutateError::NotDelivered(_)));
    assert_eq!(err.code(), "create-not-delivered");
    assert_eq!(store.count_items("INBOX").unwrap(), 1, "left as it was");
}
