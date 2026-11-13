//! The store version contract (STORAGE §4.2, §6): a draft's store opens
//! under that draft alone. A store of another draft, of a release, or one
//! lacking `format_version` (every draft before storage-draft-08), is
//! refused by every profile, the index's view of the store included,
//! naming both versions and writing nothing; so are a foreign database,
//! another algorithm and disagreeing stamps. A fresh store names this
//! draft, its identity and its derivation; the engine's first write
//! records its sync version, and a store naming another is refused.
//!
//! tests/fixtures/store-v0.5.1, store-v0.7.0 and store-v0.8.0 were written by
//! those releases (tests/fixtures/store_gen.rs, run from a `git archive` of
//! the tags, and of 3b9154a for 0.8.0, which has none). The schema of
//! tests/fixtures/storage-draft-07.sql is pimdir's 0001_init.sql at 9606b43,
//! storage-draft-07, verbatim.

#![cfg(feature = "client")]

use std::{fs, path::Path};

use io_pimdir::{
    change::PimdirWriteOp,
    client::{
        PimdirError, PimdirNotCurrent, PimdirStore, producer::PimdirProducer, reader::PimdirReader,
    },
    collection::{PimdirCheckpoint, PimdirCollectionId},
    load::PimdirLoadSelection,
    sql::FORMAT_VERSION,
    sync::SYNC_VERSION,
};
use rusqlite::Connection;

/// A copy of a checked-in fixture store, never the fixture itself.
fn fixture(name: &str) -> tempfile::TempDir {
    let from = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&from, dir.path());
    restrict(dir.path());
    dir
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn db(dir: &Path) -> Connection {
    Connection::open(dir.join("pimdir.db")).unwrap()
}

/// Restricts a store built by hand to its user, as a store this crate
/// creates is (STORAGE §18), so what a test meets is its version.
#[cfg(unix)]
fn restrict(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(dir.join("pimdir.db"), fs::Permissions::from_mode(0o600)).unwrap();
}

/// Restricts a store built by hand to its user: nothing to do where no
/// mode is checked.
#[cfg(not(unix))]
fn restrict(_dir: &Path) {}

/// A store as storage-draft-07 left it: its schema verbatim, its
/// `store_meta` row as its `init_store_meta` wrote it, a collection and a
/// queued row.
fn draft_07() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("objects")).unwrap();
    let conn = db(dir.path());
    conn.execute_batch(include_str!("fixtures/storage-draft-07.sql"))
        .unwrap();
    conn.execute_batch(
        "INSERT INTO store_meta(id, format, version, hash_algo, created_at, derivation)
         VALUES (1, 'pimdir', 1, 'blake3', '2026-10-30T09:00:00.000Z', 2);
         INSERT INTO collections(id, kind, name) VALUES ('INBOX', 'message/rfc822', 'INBOX');
         INSERT INTO queue(created_at, producer, collection, action, payload)
         VALUES ('2026-10-30T09:00:00.000Z', 'app', 'INBOX', 'remove', '{\"v\":1,\"seq\":1}');",
    )
    .unwrap();
    conn.pragma_update(None, "user_version", 1).unwrap();
    restrict(dir.path());
    dir
}

/// A store this crate created, its `store_meta` then changed by `sql`,
/// check constraints off so a value the schema refuses can be written.
fn altered(sql: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let store = PimdirStore::open(dir.path()).unwrap();
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    drop(store);
    let conn = db(dir.path());
    conn.execute_batch("PRAGMA ignore_check_constraints = ON;")
        .unwrap();
    conn.execute_batch(sql).unwrap();
    dir
}

/// The bytes a refusal must leave as they were: pimdir.db, and whatever
/// its write-ahead log holds.
fn written(dir: &Path) -> (Vec<u8>, Vec<u8>) {
    let wal = fs::read(dir.join("pimdir.db-wal")).unwrap_or_default();
    (fs::read(dir.join("pimdir.db")).unwrap(), wal)
}

/// Every profile's refusal of the store under `dir`, in turn: the owner's
/// two opens, the reader and the producer. Each is checked by `expected`,
/// and pimdir.db is left byte for byte as it was.
fn refused_by_every_profile(dir: &Path, expected: impl Fn(&PimdirError) -> bool) {
    let before = written(dir);
    let refusals = [
        ("owner", PimdirStore::open(dir).err()),
        ("open_current", PimdirStore::open_current(dir).err()),
        ("reader", PimdirReader::open(dir).err()),
        ("producer", PimdirProducer::open(dir, "app").err()),
    ];
    for (profile, refused) in refusals {
        let refused = refused.unwrap_or_else(|| panic!("{profile}: opened"));
        assert!(expected(&refused), "{profile}: {refused:?}");
    }
    assert_eq!(written(dir), before, "nothing written");
}

/// The index's view of the store refuses what every profile does, before
/// index.db is made: the indexer and the query client (SEARCH §2).
#[cfg(feature = "search")]
#[test]
fn the_index_refuses_a_store_of_another_version_and_makes_nothing() {
    use io_pimdir::client::search::PimdirIndex;

    let release = altered("UPDATE store_meta SET format_version = 'storage-v1'");
    for dir in [draft_07(), fixture("store-v0.8.0"), release] {
        let before = written(dir.path());
        for refused in [
            PimdirIndex::open(dir.path()).err(),
            PimdirIndex::open_query(dir.path()).err(),
        ] {
            assert!(
                matches!(refused, Some(PimdirError::Version { .. })),
                "{refused:?}"
            );
        }
        assert_eq!(written(dir.path()), before, "nothing written");
        assert!(!dir.path().join("index.db").exists(), "no index made");
    }
}

/// A store of a release (`storage-v1`) or of another draft is refused by
/// every profile, naming its version and this one's.
#[test]
fn a_store_of_another_version_is_refused_by_every_profile_and_never_written() {
    for marker in [
        "storage-v1",
        "storage-draft-07",
        "storage-draft-09",
        "storage-draft-10",
        "storage-draft-12",
    ] {
        let dir = altered(&format!(
            "UPDATE store_meta SET format_version = '{marker}'"
        ));
        refused_by_every_profile(dir.path(), |refused| {
            matches!(refused, PimdirError::Version { found: Some(found) } if found == marker)
                && refused.code() == "store-version"
                && refused.to_string().contains(marker)
                && refused.to_string().contains(FORMAT_VERSION)
        });
    }
}

/// A store lacking `format_version`, which storage-draft-07 and every
/// draft before it wrote, is refused by every profile, never reconciled.
#[test]
fn a_draft_07_store_is_refused_by_every_profile_and_never_written() {
    let dir = draft_07();
    refused_by_every_profile(dir.path(), |refused| {
        matches!(refused, PimdirError::Version { found: None })
            && refused.code() == "store-version"
            && refused.to_string().contains(FORMAT_VERSION)
    });
}

/// The stores io-pimdir 0.5.1, 0.7.0 and 0.8.0 wrote lack the marker, and
/// are refused by every profile as they stand: their users reinstall.
#[test]
fn a_store_a_release_of_this_crate_wrote_is_refused_by_every_profile() {
    for name in ["store-v0.5.1", "store-v0.7.0", "store-v0.8.0"] {
        let dir = fixture(name);
        refused_by_every_profile(dir.path(), |refused| {
            matches!(refused, PimdirError::Version { found: None })
        });
    }
}

/// A released store above this draft, its stamps and its schema moved on,
/// is refused for its marker before anything else is read of it.
#[test]
fn a_later_store_is_refused_for_its_marker() {
    let dir = altered(
        "UPDATE store_meta SET format_version = 'storage-v2', version = 2;
         PRAGMA user_version = 2;
         CREATE TABLE later (x INTEGER);",
    );
    refused_by_every_profile(
        dir.path(),
        |refused| matches!(refused, PimdirError::Version { found: Some(found) } if found == "storage-v2"),
    );
}

/// A pimdir.db another program wrote is not a pimdir store (§3): refused,
/// nothing in it dropped, never `store-uncreated`.
#[test]
fn a_foreign_database_is_refused_by_every_profile_and_left_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let conn = db(dir.path());
    conn.execute_batch("CREATE TABLE notes (body TEXT); INSERT INTO notes VALUES ('keep me');")
        .unwrap();
    drop(conn);
    restrict(dir.path());
    refused_by_every_profile(dir.path(), |refused| {
        matches!(refused, PimdirError::Foreign) && refused.code() == "store-foreign"
    });

    // One stamped and shaped like a store, but stating another format.
    let dir = altered("UPDATE store_meta SET format = 'other'");
    refused_by_every_profile(dir.path(), |refused| {
        matches!(refused, PimdirError::Foreign)
    });
}

/// A store naming another algorithm than BLAKE3 is refused, naming it,
/// never read as corrupt (§5, §6 step 2).
#[test]
fn a_store_of_another_algorithm_is_refused_by_every_profile() {
    let dir = altered("UPDATE store_meta SET hash_algo = 'sha256-128'");
    refused_by_every_profile(dir.path(), |refused| {
        matches!(refused, PimdirError::HashAlgo { found } if found == "sha256-128")
            && refused.code() == "store-hash-algo"
    });
}

/// `PRAGMA user_version` and `store_meta.version` disagreeing is a corrupt
/// store (§4.2).
#[test]
fn disagreeing_stamps_are_refused_by_every_profile() {
    let dir = altered("PRAGMA user_version = 2;");
    refused_by_every_profile(dir.path(), |refused| {
        matches!(
            refused,
            PimdirError::VersionMismatch {
                user_version: 2,
                store_meta: 1
            }
        ) && refused.code() == "store-corrupt"
    });
}

/// A fresh store names this draft, the three compatibility versions at 1,
/// a 32-digit id, the first incarnation, this crate's derivation and no
/// sync version, and frees pages incrementally (STORAGE §4.1, §4.2, §4.5).
#[test]
fn a_fresh_store_names_this_draft_and_an_identity() {
    let dir = tempfile::tempdir().unwrap();
    let store = PimdirStore::open(dir.path()).unwrap();
    let meta = store.store_meta().unwrap();
    assert_eq!(meta.format_version, "storage-draft-11");
    assert_eq!(
        (meta.version, meta.read_version, meta.produce_version),
        (1, 1, 1)
    );
    assert_eq!(meta.store_id.len(), 32);
    assert!(
        meta.store_id
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    );
    assert_eq!(meta.incarnation, 1);
    assert_eq!(meta.hash_algo, "blake3");
    assert_eq!(meta.derivation, 3);
    assert_eq!(meta.sync_version, None);

    let cursor = store.change_cursor().unwrap();
    assert_eq!(
        (cursor.store_id, cursor.incarnation),
        (meta.store_id.clone(), 1)
    );
    let vacuum: i64 = db(dir.path())
        .query_row("PRAGMA auto_vacuum", [], |r| r.get(0))
        .unwrap();
    assert_eq!(vacuum, 2, "INCREMENTAL");

    // NOTE: another store draws another id.
    let other = tempfile::tempdir().unwrap();
    let other = PimdirStore::open(other.path()).unwrap();
    assert_ne!(other.store_meta().unwrap().store_id, meta.store_id);
}

/// A restored copy raises its incarnation above its own and the replaced
/// store's, keeping its id, so a consumer of either sees a reset (§4.5,
/// §8).
#[test]
fn a_restored_copy_raises_its_incarnation() {
    let dir = tempfile::tempdir().unwrap();
    let store = PimdirStore::open(dir.path()).unwrap();
    let id = store.change_cursor().unwrap().store_id;

    let created = store.change_cursor().unwrap().incarnation;

    let first = store.raise_incarnation(None).unwrap();
    assert!(first > created, "above the copy's own");
    let replaced = first + 3;
    let second = store.raise_incarnation(Some(replaced)).unwrap();
    assert!(second > first && second > replaced, "above both");
    let cursor = store.change_cursor().unwrap();
    assert_eq!((cursor.store_id, cursor.incarnation), (id, second));
}

/// Two restores of one backup, the replaced store unread, answer two
/// incarnations (§8): the stride is drawn, so a consumer of one copy never
/// takes the other for the store it followed.
#[test]
fn two_restores_of_one_backup_answer_two_incarnations() {
    let backup = tempfile::tempdir().unwrap();
    drop(PimdirStore::open(backup.path()).unwrap());

    let restored = || {
        let dir = tempfile::tempdir().unwrap();
        copy_dir(backup.path(), dir.path());
        restrict(dir.path());
        let store = PimdirStore::open(dir.path()).unwrap();
        let before = store.change_cursor().unwrap();
        let raised = store.raise_incarnation(None).unwrap();
        assert!(raised > before.incarnation, "above the copy's own");
        (before.store_id, raised)
    };
    let (first, second) = (restored(), restored());
    assert_eq!(first.0, second.0, "one store, restored twice");
    assert_ne!(first.1, second.1, "never the same incarnation");
}

/// The engine's first write records its sync version (STORAGE §4.3); a
/// store whose sync state another engine version wrote is refused before
/// any verb reads or writes it (SYNC §10).
#[test]
fn the_engine_records_its_sync_version_and_refuses_another() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = PimdirCollectionId::from("INBOX");
    let checkpoint = |at: u8| PimdirWriteOp::SetCheckpoint {
        collection: inbox.clone(),
        checkpoint: PimdirCheckpoint(vec![at]),
    };

    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("imap");
    store.load(&inbox, &PimdirLoadSelection::All).unwrap();
    assert_eq!(
        store.store_meta().unwrap().sync_version,
        None,
        "a load writes nothing"
    );
    store.write(vec![checkpoint(1)]).unwrap();
    assert_eq!(
        store.store_meta().unwrap().sync_version.as_deref(),
        Some(SYNC_VERSION)
    );
    store.write(vec![checkpoint(2)]).unwrap();
    drop(store);

    db(dir.path())
        .execute("UPDATE store_meta SET sync_version = 'sync-draft-06'", [])
        .unwrap();
    let before = written(dir.path());
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("imap");
    let loaded = store.load(&inbox, &PimdirLoadSelection::All).err();
    assert!(
        matches!(&loaded, Some(PimdirError::SyncVersion { found }) if found == "sync-draft-06"),
        "{loaded:?}"
    );
    let wrote = store.write(vec![checkpoint(3)]).err().unwrap();
    assert!(
        matches!(wrote, PimdirError::SyncVersion { .. }),
        "{wrote:?}"
    );
    assert_eq!(wrote.code(), "sync-version");
    drop(store);
    assert_eq!(written(dir.path()), before, "nothing written");
}

/// An `open_current` of a database holding no schema creates none.
#[test]
fn an_empty_database_is_not_created_by_open_current() {
    let dir = tempfile::tempdir().unwrap();
    drop(db(dir.path()));
    restrict(dir.path());
    let refused = PimdirStore::open_current(dir.path()).err();
    assert!(
        matches!(
            refused,
            Some(PimdirError::NotCurrent(PimdirNotCurrent::Version {
                found: 0
            }))
        ),
        "{refused:?}"
    );
    assert_eq!(fs::metadata(dir.path().join("pimdir.db")).unwrap().len(), 0);
}
