//! # Blob directory
//!
//! The content-addressed body files beside the database (STORAGE §5):
//! sharded by hash, written atomically, read independently of the
//! SQLite connection so a body can be streamed while the store is busy.

use core::sync::atomic::{AtomicU64, Ordering};

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use std::{
    fs,
    io::{self, ErrorKind, Write},
    path::{Path, PathBuf},
};

use crate::{
    change::PimdirWriteOp,
    hash::{PimdirHashAlgo, PimdirHasher},
    object::PimdirHash,
};

/// A handle over a store's blob directory (STORAGE §5).
///
/// Bound to the hash its bodies are named by. Cheap to clone: it wraps
/// the objects/ path.
#[derive(Clone, Debug)]
pub struct PimdirBlobs {
    root: PathBuf,
    hash: PimdirHashAlgo,
}

impl PimdirBlobs {
    /// The blob handle of the store rooted at `dir`, naming bodies with `hash`.
    pub fn open(dir: impl AsRef<Path>, hash: PimdirHashAlgo) -> Self {
        Self {
            root: dir.as_ref().join("objects"),
            hash,
        }
    }

    /// The hash bodies here are named by.
    pub fn hash_algo(&self) -> PimdirHashAlgo {
        self.hash
    }

    /// The content hash of a whole body, under this store's algorithm.
    pub fn hash(&self, bytes: &[u8]) -> PimdirHash {
        self.hash.hash(bytes)
    }

    /// An incremental hasher, for a body streamed through [`writer`](Self::writer).
    pub fn hasher(&self) -> PimdirHasher {
        self.hash.hasher()
    }

    /// Where a body under `hash` lives: `objects/<h[0:2]>/<h[2:4]>/<h>` (§5).
    ///
    /// Public because the format invites a consumer to stream a body to
    /// this path and index it with a byteless `StoreObject` (§14).
    pub fn path(&self, hash: &PimdirHash) -> PathBuf {
        blob_path(&self.root, &hash.0)
    }

    /// The body stored under `hash`, or `None` when absent.
    pub fn get(&self, hash: &PimdirHash) -> io::Result<Option<Vec<u8>>> {
        match fs::read(self.path(hash)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// A stored body as a readable stream, or `None` when absent.
    pub fn reader(&self, hash: &PimdirHash) -> io::Result<Option<fs::File>> {
        match fs::File::open(self.path(hash)) {
            Ok(file) => Ok(Some(file)),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// A streaming writer for a new body: bytes go to a temporary file
    /// and reach their content-addressed path on commit, once the caller
    /// knows the hash it computed while writing.
    pub fn writer(&self) -> io::Result<PimdirBlobWriter> {
        fs::create_dir_all(&self.root)?;
        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = self.root.join(format!(".tmp-{}-{seq}", std::process::id()));
        let file = fs::File::create(&tmp)?;
        Ok(PimdirBlobWriter {
            root: self.root.clone(),
            tmp,
            file: Some(file),
            written: 0,
        })
    }

    /// Every body the blob tree holds: the files sitting at the path §5
    /// derives from their name. A period-prefixed temporary file belongs
    /// to a writer that has not committed, and a file elsewhere than its
    /// name's shard is not the store's (STORAGE §3); both are skipped.
    pub fn files(&self) -> io::Result<Vec<PimdirBlobFile>> {
        let mut files = Vec::new();
        if self.root.is_dir() {
            self.walk(&self.root, &mut files)?;
        }
        Ok(files)
    }

    fn walk(&self, dir: &Path, files: &mut Vec<PimdirBlobFile>) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }

            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                self.walk(&entry.path(), files)?;
            } else if metadata.is_file() && entry.path() == blob_path(&self.root, &name) {
                files.push(PimdirBlobFile {
                    hash: name,
                    path: entry.path(),
                    size: metadata.len(),
                });
            }
        }

        Ok(())
    }

    /// Writes every body a batch carries, ahead of the transaction that
    /// indexes them (§14): a body is immutable, so writing it early leaves
    /// at worst an orphan for the collector.
    pub(crate) fn stage(&self, ops: &[PimdirWriteOp]) -> io::Result<()> {
        for op in ops {
            if let PimdirWriteOp::StoreObject {
                object,
                body: Some(body),
            } = op
            {
                self.write(&object.hash, body)?;
            }
        }

        Ok(())
    }

    /// Writes a body atomically: temporary file, `fsync`, `rename`, then
    /// `fsync` of the shard directories the rename reached (§5). A present
    /// hash is left alone.
    pub(crate) fn write(&self, hash: &PimdirHash, body: &[u8]) -> io::Result<()> {
        let path = self.path(hash);
        if path.exists() {
            return Ok(());
        }
        let parent = path.parent().unwrap_or(&self.root);
        fs::create_dir_all(parent)?;
        let tmp = parent.join(format!(".{}.tmp", hash.0));
        {
            let mut file = fs::File::create(&tmp)?;
            file.write_all(body)?;
            file.sync_all()?;
        }
        fs::rename(&tmp, &path)?;
        sync_shards(&self.root, &path)
    }
}

/// One body as it sits in the blob tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirBlobFile {
    /// The hash its file name claims, unverified.
    pub hash: String,
    /// Where it sits.
    pub path: PathBuf,
    /// Its size on disk.
    pub size: u64,
}

/// A per-process discriminator, so concurrent writers never share a
/// staging file.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// A streaming writer for one new body, see [`PimdirBlobs::writer`].
///
/// Dropped without a commit, it removes its temporary file.
pub struct PimdirBlobWriter {
    root: PathBuf,
    tmp: PathBuf,
    file: Option<fs::File>,
    written: u64,
}

impl PimdirBlobWriter {
    /// Finalises the body under `hash`: `fsync`, then an atomic rename
    /// into its sharded path. A body already present keeps the stored
    /// copy. Returns the size written.
    pub fn commit(mut self, hash: &PimdirHash) -> io::Result<u64> {
        let file = self.file.take().expect("writer open");
        file.sync_all()?;
        drop(file);

        let path = blob_path(&self.root, &hash.0);
        if path.exists() {
            let _ = fs::remove_file(&self.tmp);
            return Ok(self.written);
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&self.tmp, &path)?;
        sync_shards(&self.root, &path)?;
        Ok(self.written)
    }
}

impl Write for PimdirBlobWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let file = self.file.as_mut().expect("writer open");
        let n = file.write(buf)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.as_mut().expect("writer open").flush()
    }
}

impl Drop for PimdirBlobWriter {
    fn drop(&mut self) {
        if self.file.is_some() {
            let _ = fs::remove_file(&self.tmp);
        }
    }
}

/// The sharded path of a blob; a hash shorter than four characters lies flat.
fn blob_path(blobs: &Path, hash: &str) -> PathBuf {
    if hash.len() >= 4 {
        blobs.join(&hash[0..2]).join(&hash[2..4]).join(hash)
    } else {
        blobs.join(hash)
    }
}

/// Flushes every directory between a blob and the root, so the rename and
/// the shard directories it may have created survive a power loss:
/// syncing the file makes its bytes durable and says nothing about the
/// name that reaches them (§5).
fn sync_shards(root: &Path, blob: &Path) -> io::Result<()> {
    let mut dir = blob.parent();
    while let Some(shard) = dir {
        fs::File::open(shard)?.sync_all()?;
        if shard == root {
            break;
        }
        dir = shard.parent();
    }
    Ok(())
}
