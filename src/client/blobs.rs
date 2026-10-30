//! # Blob directory
//!
//! The content-addressed body files beside the database (STORAGE §5):
//! sharded by hash, written atomically, read independently of the
//! SQLite connection so a body can be streamed while the store is busy.
//! [`PimdirBlobs`] reads them; [`PimdirBlobStage`], handed out by a
//! producer or an owner and holding its lock, is the only way to write
//! one (§8).

use core::sync::atomic::{AtomicU64, Ordering};

use alloc::{
    format,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};

use std::{
    fs,
    io::{self, ErrorKind, Read, Write},
    path::{Path, PathBuf},
};

use crate::{
    change::PimdirWriteOp,
    client::lock::PimdirLock,
    hash::{PimdirHashAlgo, PimdirHasher},
    object::{PimdirHash, PimdirObject},
};

/// A read-only handle over a store's blob directory (STORAGE §5).
///
/// Bound to the hash its bodies are named by. Cheap to clone: it wraps
/// the objects/ path. It writes nothing: a body enters the tree through
/// a [`PimdirBlobStage`], which holds a lock keeping the collector out.
///
/// A reader, which takes no lock, cannot write a body:
///
/// ```compile_fail
/// use io_pimdir::client::reader::PimdirReader;
///
/// let reader = PimdirReader::open("store").unwrap();
/// reader.blobs().writer();
/// ```
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

    /// An incremental hasher under this store's algorithm.
    pub fn hasher(&self) -> PimdirHasher {
        self.hash.hasher()
    }

    /// Where a body under `hash` lives: `objects/<h[0:2]>/<h[2:4]>/<h>` (§5).
    pub fn path(&self, hash: &PimdirHash) -> PathBuf {
        blob_path(&self.root, &hash.0)
    }

    /// The body stored under `hash`, checked against its name (§7), or
    /// `None` when absent: a file whose bytes do not hash to its name is
    /// no body at all, the same state as a missing one, never corrupt
    /// content handed to a parser, an index or a push.
    pub fn get(&self, hash: &PimdirHash) -> io::Result<Option<Vec<u8>>> {
        Ok(self
            .get_unverified(hash)?
            .filter(|bytes| self.hash(bytes) == *hash))
    }

    /// The bytes the file under `hash` holds, unchecked, or `None` when
    /// absent.
    fn get_unverified(&self, hash: &PimdirHash) -> io::Result<Option<Vec<u8>>> {
        match fs::read(self.path(hash)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// Unlinks the file under `hash` when its bytes do not hash to its
    /// name, under the staging lock taken exclusively, as the collector
    /// would (§7, §8), so the next write of that body writes it anew; the
    /// row stays. Answers whether it unlinked one. A producer staging
    /// holds the lock shared, and the file is then left for the next
    /// read: it is never served either way. The owner's alone.
    pub(crate) fn discard_unverified(&self, hash: &PimdirHash) -> io::Result<bool> {
        let Some(bytes) = self.get_unverified(hash)? else {
            return Ok(false);
        };
        if self.hash(&bytes) == *hash {
            return Ok(false);
        }
        let dir = self.root.parent().unwrap_or(&self.root);
        let Ok(_staging) = PimdirLock::collect(dir) else {
            return Ok(false);
        };
        match fs::remove_file(self.path(hash)) {
            Ok(()) => Ok(true),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err),
        }
    }

    /// A stored body as a readable stream, or `None` when absent. The
    /// stream hashes what it reads and verifies it at its end (§7): bytes
    /// that do not hash to `hash` fail the read reaching the length the
    /// file stated when opened, or the end of the file, as
    /// [`ErrorKind::InvalidData`], so a consumer streaming the body (an
    /// upload, an export) never hands it on as sound.
    pub fn reader(&self, hash: &PimdirHash) -> io::Result<Option<PimdirBlobReader>> {
        match fs::File::open(self.path(hash)) {
            Ok(file) => Ok(Some(PimdirBlobReader {
                len: file.metadata()?.len(),
                read: 0,
                file,
                hasher: Some(self.hasher()),
                hash: hash.clone(),
            })),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        }
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

    /// Writes a body atomically: a temporary file named uniquely to this
    /// writer in the shard directory, `fsync`, `rename`, then
    /// `fsync` of the shard directories the rename reached (§5). A present
    /// hash is left alone.
    pub(crate) fn write(&self, hash: &PimdirHash, body: &[u8]) -> io::Result<()> {
        let path = self.path(hash);
        if path.exists() {
            return Ok(());
        }
        let parent = path.parent().unwrap_or(&self.root);
        fs::create_dir_all(parent)?;
        // NOTE: unique to this writer, so two processes storing one body
        // never write, then rename, each other's bytes (§5).
        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = parent.join(format!(".{}.{}-{seq}.tmp", hash.0, std::process::id()));
        let written = fs::File::create(&tmp).and_then(|mut file| {
            file.write_all(body)?;
            file.sync_all()
        });
        if let Err(err) = written.and_then(|()| fs::rename(&tmp, &path)) {
            let _ = fs::remove_file(&tmp);
            return Err(err);
        }
        sync_shards(&self.root, &path)
    }
}

/// A stored body as a stream (STORAGE §7): it reads the file as it is and
/// hashes what it reads, failing with [`ErrorKind::InvalidData`] the read
/// that reaches the length the file stated when opened ([`len`]), or the
/// end of the file before it, when the bytes do not hash to the name; a
/// byte past that length fails the read too, the body being no longer
/// than it stated. A consumer reading the stated length, or to the end,
/// has the check run; a read of part of a body is not verified.
///
/// [`len`]: PimdirBlobReader::len
pub struct PimdirBlobReader {
    file: fs::File,
    /// `None` once the bytes were checked.
    hasher: Option<PimdirHasher>,
    hash: PimdirHash,
    /// The length the file stated when opened.
    len: u64,
    /// The bytes read so far.
    read: u64,
}

impl PimdirBlobReader {
    /// The file's metadata, as [`fs::File::metadata`].
    pub fn metadata(&self) -> io::Result<fs::Metadata> {
        self.file.metadata()
    }

    /// The body's size in bytes, as the file stated it when opened: the
    /// read reaching it verifies the body.
    pub fn len(&self) -> io::Result<u64> {
        Ok(self.len)
    }

    /// Whether the body is empty, as the file stated it when opened.
    pub fn is_empty(&self) -> io::Result<bool> {
        Ok(self.len()? == 0)
    }
}

impl PimdirBlobReader {
    /// Checks the bytes read so far against the name, once.
    fn verify(&mut self) -> io::Result<()> {
        match self.hasher.take().map(PimdirHasher::finish) {
            Some(found) if found != self.hash => Err(io::Error::new(
                ErrorKind::InvalidData,
                format!("blob {} does not hash to its name", self.hash.0),
            )),
            _ => Ok(()),
        }
    }
}

impl Read for PimdirBlobReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.file.read(buf)?;
        if read == 0 {
            if !buf.is_empty() {
                self.verify()?;
            }
            return Ok(0);
        }

        // NOTE: a body is no longer than the file stated when opened: bytes
        // past it, the check already made, are not the body.
        self.read += read as u64;
        let Some(hasher) = &mut self.hasher else {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                format!("blob {} grew past {} bytes", self.hash.0, self.len),
            ));
        };
        hasher.update(&buf[..read]);
        if self.read >= self.len {
            self.verify()?;
            if self.read > self.len {
                return Err(io::Error::new(
                    ErrorKind::InvalidData,
                    format!("blob {} grew past {} bytes", self.hash.0, self.len),
                ));
            }
        }
        Ok(read)
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

/// The write side of a store's blob directory (STORAGE §5, §8).
///
/// Handed out by a producer ([`PimdirProducer::stage_body`],
/// [`PimdirProducer::blob_writer`]) or an owner
/// ([`PimdirStore::blob_stage`]), it holds that handle's lock for as long
/// as it or a writer it opened lives: the producer's shared staging lock,
/// or the owner lock. A body written through it is never in the window a
/// collector runs in. Cheap to clone.
///
/// [`PimdirProducer::stage_body`]: crate::client::producer::PimdirProducer::stage_body
/// [`PimdirProducer::blob_writer`]: crate::client::producer::PimdirProducer::blob_writer
/// [`PimdirStore::blob_stage`]: crate::client::PimdirStore::blob_stage
#[derive(Clone)]
pub struct PimdirBlobStage {
    blobs: PimdirBlobs,
    lock: Arc<PimdirLock>,
}

impl PimdirBlobStage {
    /// The stage over `blobs`, holding `lock` while it lives.
    pub(crate) fn new(blobs: PimdirBlobs, lock: Arc<PimdirLock>) -> Self {
        Self { blobs, lock }
    }

    /// The read side of the same directory.
    pub fn blobs(&self) -> &PimdirBlobs {
        &self.blobs
    }

    /// Hashes a whole body under the store's algorithm and writes it
    /// atomically (§5), answering the object an enqueue or a byteless
    /// `StoreObject` names. A body already present keeps the stored copy.
    pub fn stage_body(&self, bytes: &[u8]) -> io::Result<PimdirObject> {
        let hash = self.blobs.hash(bytes);
        self.blobs.write(&hash, bytes)?;
        Ok(PimdirObject {
            hash,
            size: bytes.len(),
        })
    }

    /// A streaming writer for a new body: bytes go to a temporary file,
    /// hashed as they pass, and reach their content-addressed path on
    /// [`commit`](PimdirBlobWriter::commit).
    pub fn writer(&self) -> io::Result<PimdirBlobWriter> {
        let root = &self.blobs.root;
        fs::create_dir_all(root)?;
        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = root.join(format!(".tmp-{}-{seq}", std::process::id()));
        let file = fs::File::create(&tmp)?;
        Ok(PimdirBlobWriter {
            root: root.clone(),
            tmp,
            file: Some(file),
            hasher: Some(self.blobs.hasher()),
            written: 0,
            _lock: Arc::clone(&self.lock),
        })
    }
}

/// A per-process discriminator, so concurrent writers never share a
/// staging file.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// A streaming writer for one new body, see [`PimdirBlobStage::writer`].
///
/// Holds the stage's lock until it is committed or dropped. Dropped
/// without a commit, it removes its temporary file.
pub struct PimdirBlobWriter {
    root: PathBuf,
    tmp: PathBuf,
    file: Option<fs::File>,
    hasher: Option<PimdirHasher>,
    written: u64,
    _lock: Arc<PimdirLock>,
}

impl PimdirBlobWriter {
    /// Finalises the body under the hash of the bytes written: `fsync`,
    /// then an atomic rename into its sharded path (§5). A body already
    /// present keeps the stored copy. Answers the object it names.
    pub fn commit(mut self) -> io::Result<PimdirObject> {
        let file = self.file.take().expect("writer open");
        file.sync_all()?;
        drop(file);

        let hash = self.hasher.take().expect("writer open").finish();
        let object = PimdirObject {
            hash,
            size: self.written as usize,
        };

        let path = blob_path(&self.root, &object.hash.0);
        if path.exists() {
            let _ = fs::remove_file(&self.tmp);
            return Ok(object);
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&self.tmp, &path)?;
        sync_shards(&self.root, &path)?;
        Ok(object)
    }
}

impl Write for PimdirBlobWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let file = self.file.as_mut().expect("writer open");
        let n = file.write(buf)?;
        self.hasher.as_mut().expect("writer open").update(&buf[..n]);
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
