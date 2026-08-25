//! The advisory locks the store's single-owner rule is made of (spec §8).
//!
//! Two files sit beside `pimdir.db`, each locked by the handle that took it
//! and released when that handle drops:
//!
//! - `owner.lock`, held exclusively by every owning handle, so a store has at
//!   most one owner process and a second one is told so rather than made to
//!   wait behind it.
//! - `objects.lock`, held shared by every producer, so a body is never between
//!   the blob tree and the queue row that pins it while a collector runs.
//!
//! The lock lives on the open file description, which is what makes a crashed
//! owner harmless: the kernel releases it with the process, leaving a lock file
//! that locks nothing. An `O_EXCL` lock file cannot promise that, and the
//! escape hatch it needs for a stale one turns fail-fast into fail-always.

use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, PoisonError, Weak},
};

// NOTE: std grew these locks in 1.89 and its inherent methods would shadow the
// trait's, so the two calls below name the trait: `fs4` is what keeps the
// crate's stated MSRV, and it goes when that reaches 1.89.
use fs4::FileExt;

use crate::client::PimdirError;

/// The file an owning handle locks exclusively.
const OWNER: &str = "owner.lock";

/// The file a producer locks shared while it stages a body.
const OBJECTS: &str = "objects.lock";

/// The owner locks this process holds, keyed by store directory.
///
/// The rule is about processes and an advisory lock is about open file
/// descriptions, so a second handle in the same process shares the lock the
/// first one took instead of contending with itself: a two-sided sync opens one
/// handle per source and a multi-account owner one per account, and each of
/// those processes is one owner.
static OWNED: LazyLock<Mutex<HashMap<PathBuf, Weak<PimdirLock>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// An advisory lock on a store directory, held for as long as it lives.
pub struct PimdirLock {
    /// The locked file, kept because the lock is the description rather than
    /// anything written in it: closing this releases the lock, and nothing
    /// ever reads or writes its bytes.
    _file: File,
    /// The [`OWNED`] key to withdraw on drop; `None` for a lock the registry
    /// does not track, which is every shared one.
    registered: Option<PathBuf>,
}

impl PimdirLock {
    /// Takes the store's exclusive owner lock, or reports the store owned.
    ///
    /// Another process holding it is [`PimdirError::Owned`], returned
    /// immediately: a wait long enough to outlast a sync's transaction is a
    /// stall with no signal, and what to do instead (retry, back off, queue the
    /// intent, tell the user) is a policy only the program on top can pick.
    pub fn own(dir: &Path) -> Result<Arc<Self>, PimdirError> {
        let key = dir.canonicalize()?;
        let mut owned = OWNED.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(lock) = owned.get(&key).and_then(Weak::upgrade) {
            return Ok(lock);
        }

        let file = open(&dir.join(OWNER))?;
        FileExt::try_lock(&file).map_err(|_| PimdirError::Owned(dir.to_path_buf()))?;

        let lock = Arc::new(Self {
            _file: file,
            registered: Some(key.clone()),
        });
        owned.insert(key, Arc::downgrade(&lock));
        Ok(lock)
    }

    /// Takes the store's staging lock exclusively: the collector's half of the
    /// pairing below.
    ///
    /// A producer holds this lock for as long as its handle lives, so a
    /// collector that waited would wait on a program's lifetime rather than on
    /// an operation: it reports [`PimdirError::Staging`] instead, and the
    /// operator retries when the frontend is done. The owner side needs no
    /// separate acquisition: a collector runs on an owning handle, which
    /// already holds the owner lock, so no other owner can be writing while it
    /// sweeps.
    pub fn collect(dir: &Path) -> Result<Self, PimdirError> {
        let file = open(&dir.join(OBJECTS))?;
        FileExt::try_lock(&file).map_err(|_| PimdirError::Staging(dir.to_path_buf()))?;
        Ok(Self {
            _file: file,
            registered: None,
        })
    }

    /// Takes the store's shared staging lock: any number of producers hold it
    /// at once, and a collector holds it exclusively.
    ///
    /// A producer writes a body to the blob tree and then enqueues the action
    /// that pins it, and in between the body is a file nothing references. The
    /// lock spans the producer handle rather than the enqueue, so the blob
    /// write that precedes it is inside the window too. It waits rather than
    /// failing: the only thing it waits for is a collector, whose work is
    /// bounded, and failing an append because a sweep is running is not an
    /// answer a producer can do anything with.
    pub fn stage(dir: &Path) -> Result<Self, PimdirError> {
        let file = open(&dir.join(OBJECTS))?;
        FileExt::lock_shared(&file)?;
        Ok(Self {
            _file: file,
            registered: None,
        })
    }
}

impl Drop for PimdirLock {
    fn drop(&mut self) {
        let Some(key) = &self.registered else {
            return;
        };

        // NOTE: only when the entry is still this lock's. A handle taken
        // between this one losing its last reference and this line runs
        // registers its own under the same key, and that one is live.
        let mut owned = OWNED.lock().unwrap_or_else(PoisonError::into_inner);
        if owned.get(key).is_some_and(|lock| lock.strong_count() == 0) {
            owned.remove(key);
        }
    }
}

/// Opens (creating if absent) a lock file.
fn open(path: &Path) -> Result<File, PimdirError> {
    Ok(OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)?)
}
