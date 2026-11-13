//! # Modes
//!
//! A store's files are its user's alone (STORAGE §18): its directory,
//! objects/, the shard directories and quarantine/ are created `0700`,
//! every file in them `0600`, pimdir.db and index.db created so before
//! SQLite opens them, which gives their `-wal`, `-shm` and journal the
//! database's own mode. A store whose directory or pimdir.db another user
//! can read or write is refused by every profile as
//! [`PimdirError::Exposed`], and the user restricts it.
//!
//! The subdirectory local/ is the owner's own (§3): created `0700` when
//! the owner first asks for it, never defined by the format, refused by
//! the owner's open when another user can reach it, and ignored by a
//! reader and a producer.
//!
//! On Unix through the permission bits; elsewhere a directory and a file
//! take the access list their parent directory hands down, and nothing is
//! refused. The lock files are opened by the locks themselves, without
//! following a link.

use std::{
    fs::{self, OpenOptions},
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use crate::client::PimdirError;

/// The mode of every directory of a store.
#[cfg(unix)]
const DIR_MODE: u32 = 0o700;

/// The subdirectory of a store reserved for its owner's own state
/// (STORAGE §3).
pub(crate) const LOCAL: &str = "local";

/// The mode of every file of a store.
#[cfg(unix)]
const FILE_MODE: u32 = 0o600;

/// Creates `path` and every missing parent, each `0700` on Unix.
#[cfg(unix)]
pub fn create_dir_all(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    fs::DirBuilder::new()
        .recursive(true)
        .mode(DIR_MODE)
        .create(path)
}

/// Creates `path` and every missing parent, each `0700` on Unix.
#[cfg(not(unix))]
pub fn create_dir_all(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)
}

/// Creates the directory `path`, `0700` on Unix, failing as
/// [`ErrorKind::AlreadyExists`] when something holds the name.
#[cfg(unix)]
pub fn create_dir(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    fs::DirBuilder::new().mode(DIR_MODE).create(path)
}

/// Creates the directory `path`, `0700` on Unix, failing as
/// [`ErrorKind::AlreadyExists`] when something holds the name.
#[cfg(not(unix))]
pub fn create_dir(path: &Path) -> io::Result<()> {
    fs::create_dir(path)
}

/// Options opening a file of a store: one they create is `0600` on Unix.
#[cfg(unix)]
pub fn options() -> OpenOptions {
    use std::os::unix::fs::OpenOptionsExt;

    let mut options = OpenOptions::new();
    options.mode(FILE_MODE);
    options
}

/// Options opening a file of a store: one they create is `0600` on Unix.
#[cfg(not(unix))]
pub fn options() -> OpenOptions {
    OpenOptions::new()
}

/// Whether another user can read or write `path` (STORAGE §18): on Unix,
/// a permission bit of its group or of others set, or another owner than
/// this process's user. Nothing elsewhere.
#[cfg(unix)]
pub fn exposed(path: &Path) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;

    let meta = fs::metadata(path)?;
    let user = rustix::process::geteuid().as_raw();

    Ok(meta.mode() & 0o077 != 0 || meta.uid() != user)
}

/// Whether another user can read or write `path` (STORAGE §18): on Unix,
/// a permission bit of its group or of others set, or another owner than
/// this process's user. Nothing elsewhere.
#[cfg(not(unix))]
pub fn exposed(_path: &Path) -> io::Result<bool> {
    Ok(false)
}

/// Restricts the directory a store is being created in to its user,
/// `0700` on Unix: the store's creator decides its directory's mode,
/// whoever made the directory.
#[cfg(unix)]
pub(crate) fn restrict(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(dir, fs::Permissions::from_mode(DIR_MODE))
}

/// Restricts the directory a store is being created in to its user,
/// `0700` on Unix: the store's creator decides its directory's mode,
/// whoever made the directory.
#[cfg(not(unix))]
pub(crate) fn restrict(_dir: &Path) -> io::Result<()> {
    Ok(())
}

/// Creates the database file `path` empty and `0600` when absent, so the
/// SQLite opening it next creates none of another mode and gives its
/// `-wal` and `-shm` the same.
pub(crate) fn create_database(path: &Path) -> io::Result<()> {
    match options().write(true).create_new(true).open(path) {
        Ok(_) => Ok(()),
        Err(err) if err.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(err) => Err(err),
    }
}

/// Refuses the store rooted at `dir` when another user can read or write
/// its directory or its pimdir.db (STORAGE §18), naming the first.
pub(crate) fn check(dir: &Path) -> Result<(), PimdirError> {
    let db = dir.join("pimdir.db");
    for path in [dir, db.as_path()] {
        match exposed(path) {
            Ok(true) => return Err(PimdirError::Exposed(path.to_path_buf())),
            Ok(false) => (),
            Err(err) if err.kind() == ErrorKind::NotFound => (),
            Err(err) => return Err(err.into()),
        }
    }

    Ok(())
}

/// Refuses the store rooted at `dir` for its owner when its local/
/// exists and is not the owner's alone (STORAGE §3, §18): a link or a
/// file where the directory belongs, or a directory another user can
/// read or write. One that does not exist yet refuses nothing.
pub(crate) fn check_local(dir: &Path) -> Result<(), PimdirError> {
    let path = dir.join(LOCAL);
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    if !meta.is_dir() || exposed(&path)? {
        return Err(PimdirError::Exposed(path));
    }

    Ok(())
}

/// The local/ of the store rooted at `dir`, created `0700` when absent
/// (STORAGE §3, §18), answering its path; one that is not the owner's
/// alone is [`PimdirError::Exposed`], as [`check_local`] refuses it.
pub(crate) fn local(dir: &Path) -> Result<PathBuf, PimdirError> {
    let path = dir.join(LOCAL);
    match create_dir(&path) {
        Ok(()) => (),
        Err(err) if err.kind() == ErrorKind::AlreadyExists => (),
        Err(err) => return Err(err.into()),
    }
    check_local(dir)?;

    Ok(path)
}
