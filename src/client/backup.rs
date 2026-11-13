//! # Copies and the deep check
//!
//! The tools STORAGE §7 and §8 specify: a backup, a restore and the
//! store's self-check.
//!
//! A backup copies pimdir.db consistently, through `VACUUM INTO`, never
//! as a file under a running owner, then objects/, quarantine/ and
//! local/, holding the staging lock shared across the copies though it
//! writes nothing in the store, so no collector unlinks a body the copied
//! database names. A restore puts them back under the owner lock, before
//! any program opens the store, and raises the incarnation of the copy,
//! given the replaced store's when that one can still be read, so no
//! consumer takes the copy for the store it followed (§4.5). The deep
//! check reads `PRAGMA integrity_check` and `PRAGMA foreign_key_check`
//! and hashes every body an object row names against its name; it
//! reports, and moves nothing.
//!
//! The lock files and the search index are no part of a copy: a lock
//! file holds no state and is never removed nor replaced (§8), and the
//! index is derived, its indexer rebuilding it from a store whose
//! incarnation moved.

use alloc::{collections::BTreeSet, format, string::String, vec::Vec};

use std::{
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use log::{debug, warn};
use rusqlite::{Connection, OpenFlags, named_params};

use crate::{
    client::{
        PimdirError, PimdirStore, PreparedCached,
        blobs::{PimdirBlobCheck, PimdirBlobStrayKind, PimdirBlobs, sync_dir},
        lock::PimdirLock,
        modes, prepare_connection,
        reader::PimdirReader,
        rows, schema,
    },
    object::PimdirHash,
    sql,
    version::PimdirProfile,
};

/// The database of a store.
const DATABASE: &str = "pimdir.db";

/// The directory holding the bodies that failed their name (§7).
const QUARANTINE: &str = "quarantine";

/// The name a restore stages the database under, inside local/, until it
/// is renamed onto pimdir.db: the owner's own mark of work under way
/// (§3).
const STAGED: &str = "restore.db.tmp";

/// What a copy of a store's files moved: a backup's, or a restore's.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirCopyReport {
    /// Bodies of objects/ copied.
    pub objects: usize,
    /// Files of quarantine/ copied.
    pub quarantined: usize,
    /// Files of local/ copied.
    pub local: usize,
    /// The bytes those files hold, the database's left out.
    pub bytes: u64,
    /// Files the target held already under their name, left as they
    /// were: a body is never replaced (§5). None for a backup, whose
    /// target is empty.
    pub kept: usize,
    /// The entries left out: a link, a writer's temporary, a name that
    /// is not the store's, anything that is no regular file nor a
    /// directory.
    pub skipped: Vec<PathBuf>,
}

/// What a restore did (§8).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirRestoreReport {
    /// The incarnation the restored store answers, one no consumer of it
    /// recorded.
    pub incarnation: i64,
    /// The incarnation of the store the copy replaced, `None` when there
    /// was none or it could not be read.
    pub replaced: Option<i64>,
    /// The files put back.
    pub copied: PimdirCopyReport,
}

/// A row `PRAGMA foreign_key_check` answers: a reference naming no
/// parent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirForeignKeyViolation {
    /// The table holding the reference.
    pub table: String,
    /// The row holding it, `None` in a table without rowids.
    pub rowid: Option<i64>,
    /// The table it names.
    pub parent: String,
}

/// What the deep check found (§7): empty lists for a sound store.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirDeepReport {
    /// What `PRAGMA integrity_check` answered beside `ok`, a line per
    /// problem.
    pub integrity: Vec<String>,
    /// What `PRAGMA foreign_key_check` answered.
    pub foreign_keys: Vec<PimdirForeignKeyViolation>,
    /// How many object rows were checked, each body hashed.
    pub objects: usize,
    /// The object rows whose body the blob directory does not hold.
    pub missing: Vec<PimdirHash>,
    /// The object rows whose body does not hash to its name, or is a
    /// link: left where it is, an owner's read being what moves it into
    /// quarantine/.
    pub failing: Vec<PimdirHash>,
}

impl PimdirDeepReport {
    /// Whether the check found nothing.
    pub fn is_sound(&self) -> bool {
        self.integrity.is_empty()
            && self.foreign_keys.is_empty()
            && self.missing.is_empty()
            && self.failing.is_empty()
    }
}

impl PimdirReader {
    /// Copies the store into `target` (§8), a directory that does not
    /// exist yet or holds nothing: pimdir.db through `VACUUM INTO`, a
    /// consistent copy of one instant whatever the owner writes
    /// meanwhile, then every body of objects/, every file of quarantine/,
    /// which may be the only copy of a body, and local/, the owner's own
    /// state. The staging lock is held shared from before the database
    /// is copied until the last file is, so no collector unlinks a body
    /// the copied database names; a collector running is waited out.
    ///
    /// The copy is the store's user's alone (§18), directories `0700` and
    /// files `0600`, each file flushed and each directory after its
    /// files. Its database is one file, in a rollback journal mode. The
    /// lock files and the search index are not copied. A writer's
    /// temporary, a link and any entry that is not the store's are left
    /// out and named in the report.
    ///
    /// A `target` holding anything is [`PimdirError::Occupied`], nothing
    /// copied.
    pub fn backup(&self, target: impl AsRef<Path>) -> Result<PimdirCopyReport, PimdirError> {
        let target = target.as_ref();
        match fs::read_dir(target) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    return Err(PimdirError::Occupied(target.to_path_buf()));
                }
            }
            Err(err) if err.kind() == ErrorKind::NotFound => (),
            Err(err) => return Err(err.into()),
        }
        let Some(database) = target.join(DATABASE).to_str().map(String::from) else {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!("Backup path {} is not UTF-8", target.display()),
            )
            .into());
        };

        // NOTE: before the database is read, so every body the copy names
        // is still in the tree when the walk reaches it (§5, §8).
        let _staging = PimdirLock::stage(&self.dir)?;

        modes::create_dir_all(target)?;
        modes::restrict(target)?;
        // NOTE: created empty and `0600` first, which `VACUUM INTO`
        // accepts and SQLite then keeps the mode of.
        modes::create_database(Path::new(&database))?;
        debug!("copy the database into {database}");
        self.conn
            .execute("VACUUM INTO :target", named_params! { ":target": database })?;
        fs::File::open(&database)?.sync_all()?;

        let report = copy_files(&self.dir, target)?;
        sync_dir(target)?;
        debug!(
            "backed up {} bodies, {} quarantined and {} local files",
            report.objects, report.quarantined, report.local
        );

        Ok(report)
    }

    /// Checks the store against itself (§7): `PRAGMA integrity_check`,
    /// `PRAGMA foreign_key_check`, and every object row's body hashed
    /// against its name, streamed, never held whole. It reads and
    /// reports: a body failing its name is left where it is, and nothing
    /// is repaired.
    ///
    /// The cost is the store's size: every page of the database and
    /// every byte of every body are read.
    pub fn check_deep(&self) -> Result<PimdirDeepReport, PimdirError> {
        let mut report = PimdirDeepReport::default();

        let integrity: Vec<String> = rows(&self.conn, "PRAGMA integrity_check", [], |r| r.get(0))?;
        report.integrity = integrity.into_iter().filter(|line| line != "ok").collect();
        report.foreign_keys = rows(&self.conn, "PRAGMA foreign_key_check", [], |r| {
            Ok(PimdirForeignKeyViolation {
                table: r.get(0)?,
                rowid: r.get(1)?,
                parent: r.get(2)?,
            })
        })?;

        let blobs = self.blobs();
        for hash in self.indexed_hashes()? {
            let hash = PimdirHash(hash);
            report.objects += 1;
            match blobs.check(&hash)? {
                PimdirBlobCheck::Sound => (),
                PimdirBlobCheck::Missing => report.missing.push(hash),
                PimdirBlobCheck::Failing => {
                    warn!("body {} fails its name", hash.0);
                    report.failing.push(hash);
                }
            }
        }

        Ok(report)
    }
}

impl PimdirStore {
    /// Puts the backup under `backup` back as the store rooted at `dir`
    /// (§8), replacing the store there when there is one, before any
    /// program opens it: the bodies of objects/ and the files of
    /// quarantine/ copied in, local/ replaced by the backup's, then
    /// pimdir.db, whose incarnation is raised (`raise_incarnation`) before
    /// it takes its name, given the replaced store's when that one could
    /// still be read. The restored store keeps its `store_id` and answers
    /// an incarnation no consumer of it recorded (§4.5): two restores of
    /// one backup answer two.
    ///
    /// Held under the owner lock, [`PimdirError::Owned`] at once when
    /// another process holds it or a handle of this process is open, and
    /// under the staging lock taken exclusively
    /// ([`PimdirError::Staging`] while a producer stages a body). The
    /// lock files stay as they are, never removed nor replaced (§8). A
    /// backup this crate does not open (another draft's, a foreign
    /// database, a damaged schema, none at all) is refused with the
    /// failure its open answers, before anything of the store is
    /// touched.
    ///
    /// A body the store holds already under its name is kept, never
    /// replaced (§5), and the bodies the replaced store alone named stay
    /// as orphans for the collector. The old database, its WAL and its
    /// journal are removed only once every body is in place and the new
    /// one is ready, flushed, under local/; what local/ holds is replaced
    /// after that, and the new database renamed onto pimdir.db last. A
    /// restore stopped before the removal leaves the store it found,
    /// beside bodies put in; stopped after, a directory holding no
    /// pimdir.db, which no program opens as a store. Either is run
    /// again. The restored database takes the journal mode of
    /// the store it replaces, WAL when there was none to read.
    ///
    /// The search index is left as it is: its indexer finds another
    /// incarnation and rebuilds it.
    pub fn restore(
        dir: impl AsRef<Path>,
        backup: impl AsRef<Path>,
    ) -> Result<PimdirRestoreReport, PimdirError> {
        let (dir, backup) = (dir.as_ref(), backup.as_ref());

        // NOTE: the backup is judged as an owner judges a store, before
        // anything of the store it replaces is touched.
        let source = backup.join(DATABASE);
        if !source.is_file() {
            return Err(PimdirError::Uncreated);
        }
        drop(open_checked(&source)?);

        let db = dir.join(DATABASE);
        if !db.exists() {
            modes::create_dir_all(dir)?;
            modes::restrict(dir)?;
        }
        modes::check(dir)?;
        let lock = PimdirLock::own(dir)?;
        if !lock.sole() {
            return Err(PimdirError::Owned(dir.to_path_buf()));
        }
        let _staging = PimdirLock::collect(dir)?;
        let _collecting = lock.collecting();
        if lock.in_flight() {
            return Err(PimdirError::InFlight(dir.to_path_buf()));
        }

        let replaced = replaced(&db);
        let journal_mode = replaced
            .as_ref()
            .map_or("wal", |(_, journal_mode)| journal_mode.as_str());
        let replaced = replaced.as_ref().map(|(incarnation, _)| *incarnation);
        debug!("restore over incarnation {replaced:?}");

        let held = backup.join(modes::LOCAL).join(STAGED);
        if fs::symlink_metadata(&held).is_ok() {
            return Err(io::Error::new(
                ErrorKind::AlreadyExists,
                format!("Backup holds a file under {}", held.display()),
            )
            .into());
        }

        // NOTE: the bodies first, which replaces nothing: interrupted
        // here, the store found stands whole, beside orphans (§8).
        let mut copied = PimdirCopyReport::default();
        copy_bodies(backup, dir, &mut copied)?;

        // NOTE: the incarnation rises before the database takes its
        // name, so the store never exists under the backup's own.
        let local = dir.join(modes::LOCAL);
        if fs::symlink_metadata(&local).is_ok_and(|meta| !meta.is_dir()) {
            fs::remove_file(&local)?;
        }
        let staged = modes::local(dir)?.join(STAGED);
        match fs::remove_file(&staged) {
            Ok(()) => (),
            Err(err) if err.kind() == ErrorKind::NotFound => (),
            Err(err) => return Err(err.into()),
        }
        copy_file(&source, &staged)?;
        let incarnation = raise(&staged, replaced)?;
        fs::File::open(&staged)?.sync_all()?;

        // NOTE: from here no pimdir.db stands until the staged one takes
        // the name, so no program opens what is left as a store.
        for suffix in ["-wal", "-shm", "-journal", ""] {
            let stale = dir.join(format!("{DATABASE}{suffix}"));
            match fs::remove_file(&stale) {
                Ok(()) => (),
                Err(err) if err.kind() == ErrorKind::NotFound => (),
                Err(err) => return Err(err.into()),
            }
        }
        sync_dir(dir)?;

        // NOTE: local/ is the backup's, whole (§3, §8): the state an
        // owner kept beside the replaced store followed that store.
        for entry in sorted(&local)? {
            if entry == staged {
                continue;
            }
            if fs::symlink_metadata(&entry)?.is_dir() {
                fs::remove_dir_all(&entry)?;
            } else {
                fs::remove_file(&entry)?;
            }
        }
        let theirs = backup.join(modes::LOCAL);
        if fs::symlink_metadata(&theirs).is_ok_and(|meta| meta.is_dir()) {
            copy_tree(&theirs, &local, &mut copied)?;
        }

        fs::rename(&staged, &db)?;
        sync_dir(dir)?;
        sync_dir(&local)?;

        let conn = Connection::open(&db)?;
        prepare_connection(&conn)?;
        let _: String =
            conn.query_row(&format!("PRAGMA journal_mode = {journal_mode}"), [], |r| {
                r.get(0)
            })?;
        drop(conn);

        Ok(PimdirRestoreReport {
            incarnation,
            replaced,
            copied,
        })
    }
}

/// Opens the database at `path` read-only and refuses one this crate
/// does not open as an owner ([`schema::check`]).
fn open_checked(path: &Path) -> Result<Connection, PimdirError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags)?;
    prepare_connection(&conn)?;
    schema::check(&conn, PimdirProfile::Owner)?;

    Ok(conn)
}

/// The incarnation and the journal mode of the store a restore replaces,
/// `None` when no database is there or it cannot be read: a restore
/// replaces a damaged store too.
fn replaced(db: &Path) -> Option<(i64, String)> {
    if !db.is_file() {
        return None;
    }

    let read = || -> Result<(i64, String), PimdirError> {
        let conn = open_checked(db)?;
        let incarnation = conn.query_row_cached(sql::LOAD_CHANGE_CURSOR, [], |r| r.get(3))?;
        let journal_mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
        Ok((incarnation, journal_mode))
    };
    match read() {
        Ok(replaced) => Some(replaced),
        Err(err) => {
            warn!("cannot read the store being replaced: {err}");
            None
        }
    }
}

/// Raises the incarnation of the database staged at `path`
/// (`raise_incarnation`, §8), above its own and `replaced`, answering the
/// new one. The database is left one file, in a rollback journal mode,
/// whatever its header stated.
fn raise(path: &Path, replaced: Option<i64>) -> Result<i64, PimdirError> {
    let conn = Connection::open(path)?;
    prepare_connection(&conn)?;
    let _: String = conn.query_row("PRAGMA journal_mode = DELETE", [], |r| r.get(0))?;
    conn.execute_cached(
        sql::RAISE_INCARNATION,
        named_params! { ":replaced": replaced },
    )?;
    let incarnation = conn.query_row_cached(sql::LOAD_CHANGE_CURSOR, [], |r| r.get(3))?;
    conn.close().map_err(|(_, err)| err)?;

    Ok(incarnation)
}

/// Copies the files of the store rooted at `from` into the one rooted at
/// `to` (§8): the bodies of objects/, each under its shard, the files of
/// quarantine/, and local/ whole. A file `to` holds already is kept;
/// every file copied is flushed, and every directory after its files.
fn copy_files(from: &Path, to: &Path) -> Result<PimdirCopyReport, PimdirError> {
    let mut report = PimdirCopyReport::default();
    copy_bodies(from, to, &mut report)?;

    let local = from.join(modes::LOCAL);
    if fs::symlink_metadata(&local).is_ok_and(|meta| meta.is_dir()) {
        copy_tree(&local, &to.join(modes::LOCAL), &mut report)?;
    }
    sync_dir(to)?;

    Ok(report)
}

/// Copies the bodies of the store rooted at `from` into the one rooted
/// at `to` (§8): those of objects/, each under its shard, and the files
/// of quarantine/. A file `to` holds already is kept, so nothing `to`
/// held is replaced; every file copied is flushed, and every directory
/// after its files.
fn copy_bodies(from: &Path, to: &Path, report: &mut PimdirCopyReport) -> Result<(), PimdirError> {
    let scan = PimdirBlobs::open(from).scan()?;
    let objects = to.join("objects");
    modes::create_dir_all(&objects)?;
    let mut shards: BTreeSet<PathBuf> = BTreeSet::new();
    for body in scan.files {
        let shard = objects.join(&body.hash[..2]);
        if !shards.contains(&shard) {
            match modes::create_dir(&shard) {
                Ok(()) => (),
                Err(err) if err.kind() == ErrorKind::AlreadyExists => (),
                Err(err) => return Err(err.into()),
            }
            shards.insert(shard.clone());
        }
        match copy_file(&body.path, &shard.join(&body.hash))? {
            Some(bytes) => {
                report.objects += 1;
                report.bytes += bytes;
            }
            None => report.kept += 1,
        }
    }
    for shard in &shards {
        sync_dir(shard)?;
    }
    sync_dir(&objects)?;
    report.skipped.extend(
        scan.strays
            .into_iter()
            .filter(|stray| stray.kind != PimdirBlobStrayKind::Temporary)
            .map(|stray| stray.path),
    );

    let quarantine = from.join(QUARANTINE);
    if fs::symlink_metadata(&quarantine).is_ok_and(|meta| meta.is_dir()) {
        let target = to.join(QUARANTINE);
        modes::create_dir_all(&target)?;
        for entry in sorted(&quarantine)? {
            if !fs::symlink_metadata(&entry)?.is_file() {
                report.skipped.push(entry);
                continue;
            }
            let Some(name) = entry.file_name() else {
                continue;
            };
            match copy_file(&entry, &target.join(name))? {
                Some(bytes) => {
                    report.quarantined += 1;
                    report.bytes += bytes;
                }
                None => report.kept += 1,
            }
        }
        sync_dir(&target)?;
    }
    sync_dir(to)?;

    Ok(())
}

/// Copies the directory `from` into `to`, every directory under it and
/// every regular file, counted as local files; anything else is skipped.
fn copy_tree(from: &Path, to: &Path, report: &mut PimdirCopyReport) -> Result<(), PimdirError> {
    modes::create_dir_all(to)?;
    for entry in sorted(from)? {
        let Some(name) = entry.file_name() else {
            continue;
        };
        let meta = fs::symlink_metadata(&entry)?;
        if meta.is_dir() {
            copy_tree(&entry, &to.join(name), report)?;
        } else if meta.is_file() {
            match copy_file(&entry, &to.join(name))? {
                Some(bytes) => {
                    report.local += 1;
                    report.bytes += bytes;
                }
                None => report.kept += 1,
            }
        } else {
            report.skipped.push(entry);
        }
    }
    sync_dir(to)?;

    Ok(())
}

/// The entries of the directory `dir`, by name.
fn sorted(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir)? {
        entries.push(entry?.path());
    }
    entries.sort();

    Ok(entries)
}

/// Copies the regular file `from` to `to`, created `0600` and flushed,
/// answering the bytes copied; `None`, nothing written, when `to` exists
/// already: a file of a store is never replaced (§5).
fn copy_file(from: &Path, to: &Path) -> io::Result<Option<u64>> {
    let mut target = match modes::options().write(true).create_new(true).open(to) {
        Ok(target) => target,
        Err(err) if err.kind() == ErrorKind::AlreadyExists => return Ok(None),
        Err(err) => return Err(err),
    };
    let copied = io::copy(&mut fs::File::open(from)?, &mut target).and_then(|bytes| {
        target.sync_all()?;
        Ok(bytes)
    });
    if copied.is_err() {
        let _ = fs::remove_file(to);
    }

    copied.map(Some)
}
