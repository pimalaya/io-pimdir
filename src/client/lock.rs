//! # Locks
//!
//! The advisory locks the store's single-owner rule is made of (STORAGE §8).
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
//! The lock lives on the open file description, which is what makes a
//! crashed owner harmless: the kernel releases it with the process,
//! leaving a lock file that locks nothing. An `O_EXCL` lock file cannot
//! promise that, and the escape hatch it needs for a stale one turns
//! fail-fast into fail-always.

use core::sync::atomic::{AtomicUsize, Ordering};

use std::{
    collections::HashMap,
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{
        Arc, LazyLock, Mutex, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard, TryLockError,
    },
};

// NOTE: std grew these locks in 1.89 and its inherent methods would
// shadow the trait's, so the two calls below name the trait. `fs4` keeps
// the crate's stated MSRV, and goes when that reaches 1.89.
use fs4::FileExt;

use crate::client::PimdirError;

/// The file an owning handle locks exclusively.
const OWNER: &str = "owner.lock";

/// The file a producer locks shared while it stages a body.
const OBJECTS: &str = "objects.lock";

/// The owner locks this process holds, keyed by store directory.
///
/// The rule is about processes and an advisory lock is about open file
/// descriptions, so a second handle in the same process shares the lock
/// the first took instead of contending with itself: a two-sided sync
/// opens one handle per source, and that process is still one owner.
///
/// The registry owns the description and counts the handles sharing it,
/// rather than tracking a `Weak`: a strong count reaches zero the moment
/// the last handle drops, while the file it named stays open until that
/// drop returns, so a handle taken in between would find no entry, open
/// a second description and `flock` itself out of its own store. Counting
/// here makes the release and the next acquisition one critical section.
static OWNED: LazyLock<Mutex<HashMap<PathBuf, Owned>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// One store's owner lock and the handles sharing it.
struct Owned {
    /// The locked description. Closing it is what releases the lock, so
    /// it is dropped inside the registry's critical section and never
    /// merely unreferenced.
    _file: File,
    handles: usize,
    /// What the process's own writers, verbs and collector serialise on.
    shared: Arc<Shared>,
}

/// What the handles of one owning process share (STORAGE §8): the owner
/// lock excludes other processes and nothing inside its own.
#[derive(Default)]
struct Shared {
    /// Held shared by a writer from blob staging through commit, and
    /// exclusively by the collector across its row deletion and its file
    /// walk.
    writers: RwLock<()>,
    /// How many verbs are between two of their chunks, and how many of
    /// the owner's blob streams are open: a body streamed in one chunk
    /// has no pointer until the next, so the collector does not run while
    /// any is (STORAGE §5). Raised under `writers` held shared and read
    /// by the collector under it held exclusively, so none starts while
    /// a collection runs.
    running: AtomicUsize,
}

/// An advisory lock on a store directory, held for as long as it lives.
pub struct PimdirLock {
    /// The locked file for a lock the registry does not track, which is
    /// every staging one. Kept because the lock is the description rather
    /// than anything written in it.
    ///
    /// `None` for an owner lock, whose description belongs to [`OWNED`].
    /// Exactly one of this and `registered` is set.
    _file: Option<File>,
    /// The [`OWNED`] key whose count to drop on release.
    registered: Option<PathBuf>,
    /// What the handles sharing an owner lock serialise on. Unused on a
    /// staging lock.
    shared: Arc<Shared>,
}

/// A verb in flight, counted until it drops (STORAGE §5).
pub struct PimdirRunning<'a>(&'a Shared);

impl Drop for PimdirRunning<'_> {
    fn drop(&mut self) {
        self.0.running.fetch_sub(1, Ordering::AcqRel);
    }
}

/// A blob stream of the owner's in flight, counted as a verb is until it
/// drops (STORAGE §5): its temporary file is no crash leftover, and the
/// body it commits has no pointer yet.
pub(crate) struct PimdirStreaming(Arc<PimdirLock>);

impl Drop for PimdirStreaming {
    fn drop(&mut self) {
        self.0.shared.running.fetch_sub(1, Ordering::AcqRel);
    }
}

impl PimdirLock {
    /// Takes the store's exclusive owner lock, or reports the store owned.
    ///
    /// Another process holding it is [`PimdirError::Owned`], returned
    /// immediately: a wait long enough to outlast a sync's transaction is
    /// a stall with no signal, and what to do instead is a policy only
    /// the program on top can pick.
    pub fn own(dir: &Path) -> Result<Arc<Self>, PimdirError> {
        let key = dir.canonicalize()?;
        let mut owned = OWNED.lock().unwrap_or_else(PoisonError::into_inner);

        let shared = match owned.get_mut(&key) {
            Some(entry) => {
                entry.handles += 1;
                Arc::clone(&entry.shared)
            }
            None => {
                let file = open(&dir.join(OWNER))?;
                FileExt::try_lock(&file).map_err(|_| PimdirError::Owned(dir.to_path_buf()))?;
                let shared = Arc::default();
                owned.insert(
                    key.clone(),
                    Owned {
                        _file: file,
                        handles: 1,
                        shared: Arc::clone(&shared),
                    },
                );
                shared
            }
        };

        Ok(Arc::new(Self {
            _file: None,
            registered: Some(key),
            shared,
        }))
    }

    /// Takes the store's staging lock exclusively: the collector's half of the
    /// pairing below.
    ///
    /// A producer holds this lock for as long as its handle lives, so a
    /// collector that waited would wait on a program's lifetime rather
    /// than on an operation: it reports [`PimdirError::Staging`] instead.
    /// The owner side needs no separate acquisition, a collector running
    /// on a handle that already holds the owner lock.
    pub fn collect(dir: &Path) -> Result<Self, PimdirError> {
        let file = open(&dir.join(OBJECTS))?;
        FileExt::try_lock(&file).map_err(|_| PimdirError::Staging(dir.to_path_buf()))?;
        Ok(Self {
            _file: Some(file),
            registered: None,
            shared: Arc::default(),
        })
    }

    /// Takes the store's shared staging lock: any number of producers hold it
    /// at once, and a collector holds it exclusively.
    ///
    /// A producer writes a body to the blob tree and then enqueues the
    /// action that pins it, and in between the body is a file nothing
    /// references. The producer holds it from the first body it stages
    /// until a row pins every one, so the blob write is inside the window
    /// too, and an idle producer holds none. It waits
    /// rather than failing: the only thing it waits for is a collector,
    /// whose work is bounded.
    pub fn stage(dir: &Path) -> Result<Self, PimdirError> {
        let file = open(&dir.join(OBJECTS))?;
        FileExt::lock_shared(&file)?;
        Ok(Self {
            _file: Some(file),
            registered: None,
            shared: Arc::default(),
        })
    }

    /// Takes the store's shared staging lock without waiting: a collector
    /// holding it exclusively is [`PimdirError::Busy`], reported at once. What
    /// a producer's probe takes and releases.
    pub fn try_stage(dir: &Path) -> Result<Self, PimdirError> {
        let file = open(&dir.join(OBJECTS))?;
        FileExt::try_lock_shared(&file).map_err(|_| PimdirError::Busy)?;
        Ok(Self {
            _file: Some(file),
            registered: None,
            shared: Arc::default(),
        })
    }

    /// Holds the process's writer lock shared, for a write from its blob
    /// staging through its commit: what keeps the collector out of the
    /// window between a body and its row (STORAGE §8).
    pub fn writing(&self) -> RwLockReadGuard<'_, ()> {
        self.shared
            .writers
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Holds the process's writer lock exclusively, for the collector's
    /// row deletion and file walk: no writer of this process is between a
    /// body and its row while it runs (STORAGE §5, §8).
    pub fn collecting(&self) -> RwLockWriteGuard<'_, ()> {
        self.shared
            .writers
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// [`collecting`](Self::collecting) without waiting, `None` while a
    /// writer of this process holds it, this thread's own included.
    pub(crate) fn try_collecting(&self) -> Option<RwLockWriteGuard<'_, ()>> {
        match self.shared.writers.try_write() {
            Ok(guard) => Some(guard),
            Err(TryLockError::Poisoned(err)) => Some(err.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        }
    }

    /// Whether this is an owner lock, not a staging one (STORAGE §8).
    pub(crate) fn owns(&self) -> bool {
        self.registered.is_some()
    }

    /// Counts a verb in flight until the guard drops, so the collector
    /// knows a body may be streamed with no pointer yet (STORAGE §5).
    ///
    /// Waits out a collection of this process: the count is raised under
    /// the writer lock held shared, so a collector, which reads it under
    /// that lock held exclusively, never runs beside a verb it did not
    /// see. A thread holding [`writing`](Self::writing) does not call it.
    pub fn running(&self) -> PimdirRunning<'_> {
        self.count();
        PimdirRunning(&self.shared)
    }

    /// Counts a blob stream in flight until the guard drops, as
    /// [`running`](Self::running) counts a verb.
    pub(crate) fn streaming(self: &Arc<Self>) -> PimdirStreaming {
        self.count();
        PimdirStreaming(Arc::clone(self))
    }

    /// Raises the in-flight count under the writer lock held shared.
    fn count(&self) {
        let _writing = self.writing();
        self.shared.running.fetch_add(1, Ordering::AcqRel);
    }

    /// Whether a verb of this process is between two of its chunks, or a
    /// blob stream of its owner open. A collector reads it once it holds
    /// [`collecting`](Self::collecting): read before, a verb starting in
    /// between would go unseen.
    pub fn in_flight(&self) -> bool {
        self.shared.running.load(Ordering::Acquire) > 0
    }
}

impl Drop for PimdirLock {
    fn drop(&mut self) {
        let Some(key) = &self.registered else {
            return;
        };

        // NOTE: the release and the close are one critical section:
        // removing the entry drops the description it holds before the
        // mutex is handed on, so the next `own` cannot find the store
        // unregistered and still locked.
        let mut owned = OWNED.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(entry) = owned.get_mut(key) else {
            return;
        };

        entry.handles -= 1;
        if entry.handles == 0 {
            owned.remove(key);
        }
    }
}

/// Opens (creating if absent, `0600`) a lock file without following a
/// link (STORAGE §18): `O_NOFOLLOW` refuses one, which is
/// [`PimdirError::Exposed`] naming it, since a process able to read the
/// store directory could otherwise point the lock at a file it holds.
#[cfg(unix)]
pub(crate) fn open(path: &Path) -> Result<File, PimdirError> {
    use rustix::fs::{Mode, OFlags};

    let flags = OFlags::CREATE | OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    match rustix::fs::open(path, flags, Mode::RUSR | Mode::WUSR) {
        Ok(fd) => Ok(File::from(fd)),
        Err(_) if is_link(path) => Err(PimdirError::Exposed(path.to_path_buf())),
        Err(err) => Err(std::io::Error::from(err).into()),
    }
}

/// Opens (creating if absent) a lock file without following a link
/// (STORAGE §18): `FILE_FLAG_OPEN_REPARSE_POINT` opens a link itself rather
/// than its target, and one opened so is [`PimdirError::Exposed`] naming
/// it.
#[cfg(windows)]
pub(crate) fn open(path: &Path) -> Result<File, PimdirError> {
    use std::os::windows::fs::OpenOptionsExt;

    /// `FILE_FLAG_OPEN_REPARSE_POINT` of winbase.h, which std does not name.
    const OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    let file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(path)?;
    if file.metadata()?.file_type().is_symlink() {
        return Err(PimdirError::Exposed(path.to_path_buf()));
    }

    Ok(file)
}

/// Opens (creating if absent) a lock file, refusing a link found there
/// before the open (STORAGE §18) where no link-free open exists.
#[cfg(not(any(unix, windows)))]
pub(crate) fn open(path: &Path) -> Result<File, PimdirError> {
    if is_link(path) {
        return Err(PimdirError::Exposed(path.to_path_buf()));
    }

    Ok(fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)?)
}

/// Whether a symbolic link sits at `path`.
#[cfg(not(windows))]
fn is_link(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

#[cfg(test)]
mod tests {
    use alloc::{format, string::String, vec::Vec};
    use std::{sync::Arc, thread};

    use super::PimdirLock;

    /// Handing the owner role over inside one process never reports the store
    /// owned by somebody else.
    ///
    /// The lock is registered per store directory and shared, so the last
    /// handle to drop releases it. Were the registry entry observable as
    /// gone while the description it named is still open, the next `own`
    /// would get a fresh descriptor and `flock` would refuse it against
    /// this process's own: an `Owned` naming a store nobody else holds.
    #[test]
    fn a_handover_within_one_process_is_never_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = Arc::new(dir.path().to_path_buf());

        let threads: Vec<_> = (0..4)
            .map(|_| {
                let path = Arc::clone(&path);
                thread::spawn(move || {
                    for _ in 0..20_000 {
                        PimdirLock::own(&path).map_err(|err| format!("{err}"))?;
                    }
                    Ok::<(), String>(())
                })
            })
            .collect();

        for thread in threads {
            thread.join().unwrap().expect("a handover was refused");
        }
    }
}
