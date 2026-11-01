//! # Blob directory
//!
//! The content-addressed body files beside the database (STORAGE §5):
//! one shard level, a name checked before it becomes a path, written
//! without ever replacing a file, opened without following a link, read
//! independently of the SQLite connection so a body can be streamed while
//! the store is busy, and checked against its name before it is used
//! (§7). [`PimdirBlobs`] reads them; [`PimdirBlobStage`], handed out by a
//! producer or an owner and holding its lock, is the only way to write one
//! (§8). A body failing its name is the owner's to move into quarantine/,
//! never unlinked.

use core::sync::atomic::{AtomicU64, Ordering};

use alloc::{
    format,
    string::{String, ToString},
    sync::Arc,
    vec,
    vec::Vec,
};

use std::{
    fs,
    io::{self, ErrorKind, Read, Seek, Write},
    path::{Path, PathBuf},
    process,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    change::PimdirWriteOp,
    client::{lock::PimdirLock, modes},
    hash::{self, PimdirHasher},
    object::{self, PimdirHash, PimdirObject},
};

/// The directory beside objects/ holding the bodies that failed their
/// name (STORAGE §3, §7).
const QUARANTINE: &str = "quarantine";

/// How many bytes a whole read hashes at a time.
const CHUNK: usize = 64 * 1024;

/// A read-only handle over a store's blob directory (STORAGE §5).
///
/// Cheap to clone: it wraps the objects/ path. It writes nothing: a body
/// enters the tree through a [`PimdirBlobStage`], which holds a lock
/// keeping the collector out.
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
}

impl PimdirBlobs {
    /// The blob handle of the store rooted at `dir`.
    pub fn open(dir: impl AsRef<Path>) -> Self {
        Self {
            root: dir.as_ref().join("objects"),
        }
    }

    /// The content hash of a whole body (§5).
    pub fn hash(&self, bytes: &[u8]) -> PimdirHash {
        hash::hash(bytes)
    }

    /// An incremental hasher (§5).
    pub fn hasher(&self) -> PimdirHasher {
        PimdirHasher::new()
    }

    /// Where a body under `hash` lives: objects/, the shard directory its
    /// first two characters name, then the whole name (§5). A hash that is
    /// no well-formed name ([`PimdirHash::is_name`]) is refused as
    /// [`ErrorKind::InvalidInput`], no path built from it.
    pub fn path(&self, hash: &PimdirHash) -> io::Result<PathBuf> {
        let (shard, name) = self.locate(hash)?;
        Ok(shard.join(name))
    }

    /// The body stored under `hash`, read whole through one descriptor and
    /// checked against its name (§7), or `None` when absent: a file whose
    /// bytes do not hash to its name, or a link where a shard or the body
    /// belongs, is no body at all, the same state as a missing one, never
    /// content handed to a parser, an index or a push. A reader or a
    /// producer finding one moves nothing; the owner's reads move it into
    /// quarantine/.
    pub fn get(&self, hash: &PimdirHash) -> io::Result<Option<Vec<u8>>> {
        let PimdirBlobOpen::Body(mut file) = self.open_body(hash)? else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;

        Ok((self.hash(&bytes) == *hash).then_some(bytes))
    }

    /// The body stored under `hash` as an open file, already read whole
    /// through that descriptor and checked against its name, then rewound,
    /// or `None` when absent or failing (§7). What serves a range of a
    /// body (an attachment by offset, a message's head) or copies it out:
    /// the check covers every read through this file while it stays open,
    /// and nothing is recorded, so the next open checks again.
    pub fn verified(&self, hash: &PimdirHash) -> io::Result<Option<fs::File>> {
        let PimdirBlobOpen::Body(mut file) = self.open_body(hash)? else {
            return Ok(None);
        };
        if hash_file(&mut file)? != *hash {
            return Ok(None);
        }
        file.rewind()?;

        Ok(Some(file))
    }

    /// A stored body as a readable stream, or `None` when absent or behind
    /// a link. The stream hashes what it reads and verifies it at its end
    /// (§7): bytes that do not hash to `hash` fail the read reaching the
    /// length the file stated when opened, or the end of the file, as
    /// [`ErrorKind::InvalidData`], so a consumer streaming the body (an
    /// upload, an export) fails before it commits anything built from it.
    pub fn reader(&self, hash: &PimdirHash) -> io::Result<Option<PimdirBlobReader>> {
        let PimdirBlobOpen::Body(file) = self.open_body(hash)? else {
            return Ok(None);
        };

        Ok(Some(PimdirBlobReader {
            len: file.metadata()?.len(),
            read: 0,
            file,
            hasher: Some(self.hasher()),
            hash: hash.clone(),
        }))
    }

    /// What objects/ holds (§5): every body sitting where its name puts
    /// it, and every other entry, left in place and reported. A body is a
    /// regular file, read without following a link, whose well-formed name
    /// begins with the two characters of its shard, itself a directory and
    /// no link named by two characters of the alphabet. quarantine/ is not
    /// walked (§7).
    pub fn scan(&self) -> io::Result<PimdirBlobScan> {
        let mut scan = PimdirBlobScan::default();
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(scan),
            Err(err) => return Err(err),
        };

        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            // NOTE: a directory entry's type is the entry's own, never a
            // link's target.
            let kind = entry.file_type()?;
            if kind.is_dir() && is_shard(&name) {
                self.scan_shard(&entry.path(), &name, &mut scan)?;
            } else {
                scan.strays.push(stray(entry.path(), &name, kind));
            }
        }

        Ok(scan)
    }

    /// Adds the bodies and the strays of one shard directory to `scan`.
    fn scan_shard(&self, shard: &Path, prefix: &str, scan: &mut PimdirBlobScan) -> io::Result<()> {
        for entry in fs::read_dir(shard)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            let kind = entry.file_type()?;
            if kind.is_file() && object::is_name(&name) && name.starts_with(prefix) {
                scan.files.push(PimdirBlobFile {
                    size: entry.metadata()?.len(),
                    path: entry.path(),
                    hash: name,
                });
            } else {
                scan.strays.push(stray(entry.path(), &name, kind));
            }
        }

        Ok(())
    }

    /// Checks the body under `hash` against its name, streamed through one
    /// descriptor, never held whole (§7).
    pub(crate) fn check(&self, hash: &PimdirHash) -> io::Result<PimdirBlobCheck> {
        Ok(match self.open_body(hash)? {
            PimdirBlobOpen::Body(mut file) => match hash_file(&mut file)? == *hash {
                true => PimdirBlobCheck::Sound,
                false => PimdirBlobCheck::Failing,
            },
            PimdirBlobOpen::Stray(_) => PimdirBlobCheck::Failing,
            PimdirBlobOpen::Missing => PimdirBlobCheck::Missing,
        })
    }

    /// Moves what sits where the body under `hash` belongs into
    /// quarantine/ when it fails its name: a file whose bytes do not hash
    /// to it, or a link at its shard or its own path (§5, §7). Checked
    /// again here, under the caller's locks, and moved under its name, a
    /// period and a suffix unique to the move, never replacing a
    /// quarantined file nor unlinked. Answers where it went, `None` when
    /// the body is sound or absent.
    ///
    /// The owner's alone, holding the staging lock exclusively and
    /// serialised against its own writers, as the collector is (§8).
    pub(crate) fn quarantine(&self, hash: &PimdirHash) -> io::Result<Option<PathBuf>> {
        let failing = match self.open_body(hash)? {
            PimdirBlobOpen::Body(mut file) => {
                if hash_file(&mut file)? == *hash {
                    return Ok(None);
                }
                self.path(hash)?
            }
            PimdirBlobOpen::Stray(path) => path,
            PimdirBlobOpen::Missing => return Ok(None),
        };

        let store = self.root.parent().unwrap_or(&self.root);
        let quarantine = store.join(QUARANTINE);
        let created = match modes::create_dir(&quarantine) {
            Ok(()) => true,
            Err(err) if err.kind() == ErrorKind::AlreadyExists => false,
            Err(err) => return Err(err),
        };

        let name = failing
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        let target = loop {
            let target = quarantine.join(format!("{name}.{}", unique()));
            match fs::symlink_metadata(&target) {
                Err(err) if err.kind() == ErrorKind::NotFound => break target,
                Err(err) => return Err(err),
                Ok(_) => continue,
            }
        };
        fs::rename(&failing, &target)?;

        sync_dir(&quarantine)?;
        if let Some(parent) = failing.parent() {
            sync_dir(parent)?;
        }
        if created {
            sync_dir(store)?;
        }

        Ok(Some(target))
    }

    /// The files quarantine/ holds, which nothing reads as bodies and only
    /// the user empties (§7).
    pub fn quarantined(&self) -> io::Result<Vec<PathBuf>> {
        let store = self.root.parent().unwrap_or(&self.root);
        let entries = match fs::read_dir(store.join(QUARANTINE)) {
            Ok(entries) => entries,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };

        let mut quarantined = Vec::new();
        for entry in entries {
            quarantined.push(entry?.path());
        }
        quarantined.sort();

        Ok(quarantined)
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

    /// Writes a body without ever replacing a file (§5): a temporary file
    /// named uniquely to this writer in the shard directory, `fsync`, a
    /// hard link onto the final name, then the temporary unlinked and the
    /// shard directory flushed, objects/ too when the shard is new. A name
    /// already taken keeps the file there, whose bytes are checked when
    /// read; any other failure fails the write.
    pub(crate) fn write(&self, hash: &PimdirHash, body: &[u8]) -> io::Result<()> {
        let (shard, name) = self.locate(hash)?;
        let path = shard.join(name);
        // NOTE: the shard first, so a name is never looked up through a
        // link where the shard belongs.
        let created = self.shard(&shard)?;
        if fs::symlink_metadata(&path).is_ok() {
            return Ok(());
        }

        // NOTE: unique to this writer, so two processes storing one body
        // never write, then link, each other's bytes (§5).
        let tmp = shard.join(format!(".{name}.{}.tmp", unique()));
        let written = modes::options()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .and_then(|mut file| {
                file.write_all(body)?;
                file.sync_all()
            });
        if let Err(err) = written {
            let _ = fs::remove_file(&tmp);
            return Err(err);
        }

        self.settle(&tmp, &shard, &path, created)
    }

    /// Moves a written temporary onto its final name without replacing a
    /// file there, `link(2)` then `unlink(2)` of the temporary, and
    /// flushes what the move changed (§5). A name already taken discards
    /// the temporary and keeps the file.
    fn settle(&self, tmp: &Path, shard: &Path, path: &Path, created: bool) -> io::Result<()> {
        let linked = fs::hard_link(tmp, path);
        let removed = fs::remove_file(tmp);
        match linked {
            Ok(()) => removed?,
            Err(err) if err.kind() == ErrorKind::AlreadyExists => return Ok(()),
            Err(err) => return Err(err),
        }

        sync_dir(shard)?;
        if created {
            sync_dir(&self.root)?;
        }

        Ok(())
    }

    /// Makes sure `shard` is a directory, answering whether it created it:
    /// a link or a file where a shard belongs holds no body and takes none
    /// (§5).
    fn shard(&self, shard: &Path) -> io::Result<bool> {
        match modes::create_dir(shard) {
            Ok(()) => return Ok(true),
            Err(err) if err.kind() == ErrorKind::AlreadyExists => (),
            Err(err) if err.kind() == ErrorKind::NotFound => {
                modes::create_dir_all(&self.root)?;
                return self.shard(shard);
            }
            Err(err) => return Err(err),
        }

        if !fs::symlink_metadata(shard)?.is_dir() {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                format!("Shard {} is no directory", shard.display()),
            ));
        }

        Ok(false)
    }

    /// The shard directory and the name of the body under `hash`, the
    /// name checked first (§5).
    fn locate<'a>(&self, hash: &'a PimdirHash) -> io::Result<(PathBuf, &'a str)> {
        if !hash.is_name() {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!("Invalid object name {:?}", hash.0),
            ));
        }

        Ok((self.root.join(&hash.0[..2]), &hash.0))
    }

    /// Opens the body under `hash` without following a link at its shard
    /// or its own path (§5): what every read of a body goes through.
    fn open_body(&self, hash: &PimdirHash) -> io::Result<PimdirBlobOpen> {
        let (shard, name) = self.locate(hash)?;
        let path = shard.join(name);

        match open_unfollowed(&shard, name) {
            Ok(file) if file.metadata()?.is_file() => Ok(PimdirBlobOpen::Body(file)),
            Ok(_) => Ok(PimdirBlobOpen::Stray(path)),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(PimdirBlobOpen::Missing),
            Err(err) => {
                // NOTE: a link refused by the open, or a file where the
                // shard belongs, is told from a failure of the disk by
                // what sits there.
                if fs::symlink_metadata(&shard).is_ok_and(|meta| !meta.is_dir()) {
                    return Ok(PimdirBlobOpen::Stray(shard));
                }
                if fs::symlink_metadata(&path).is_ok_and(|meta| !meta.is_file()) {
                    return Ok(PimdirBlobOpen::Stray(path));
                }
                Err(err)
            }
        }
    }
}

/// What objects/ holds, see [`PimdirBlobs::scan`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirBlobScan {
    /// The bodies, each where its name puts it.
    pub files: Vec<PimdirBlobFile>,
    /// Every other entry, left in place.
    pub strays: Vec<PimdirBlobStray>,
}

/// One body as it sits in the blob tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirBlobFile {
    /// The hash its file name claims, well-formed, its bytes unverified.
    pub hash: String,
    /// Where it sits.
    pub path: PathBuf,
    /// Its size on disk.
    pub size: u64,
}

/// An entry of objects/ that is no body: the collector leaves it in place
/// and reports it (§5).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirBlobStray {
    /// Where it sits.
    pub path: PathBuf,
    /// What it is.
    pub kind: PimdirBlobStrayKind,
}

/// What a stray entry of objects/ is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PimdirBlobStrayKind {
    /// A period-prefixed temporary file, a writer's.
    Temporary,
    /// A symbolic link, which holds no body.
    Link,
    /// Any other entry: a name that is not the store's, a file outside its
    /// shard, or a directory where a body belongs.
    Foreign,
}

/// A stored body as a stream (STORAGE §7): it reads the file as it is and
/// hashes what it reads, failing with [`ErrorKind::InvalidData`] the read
/// that reaches the length the file stated when opened ([`len`]), or the
/// end of the file before it, when the bytes do not hash to the name; a
/// byte past that length fails the read too, the body being no longer
/// than it stated. A consumer reading the stated length, or to the end,
/// has the check run; a read of part of a body is not verified, which
/// [`PimdirBlobs::verified`] is for.
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
                format!("Blob {} does not hash to its name", self.hash.0),
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
                format!("Blob {} grew past {} bytes", self.hash.0, self.len),
            ));
        };
        hasher.update(&buf[..read]);
        if self.read >= self.len {
            self.verify()?;
            if self.read > self.len {
                return Err(io::Error::new(
                    ErrorKind::InvalidData,
                    format!("Blob {} grew past {} bytes", self.hash.0, self.len),
                ));
            }
        }
        Ok(read)
    }
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

    /// Hashes a whole body and writes it without replacing a file (§5),
    /// answering the object an enqueue or a byteless `StoreObject` names.
    /// A body already present keeps the stored copy.
    pub fn stage_body(&self, bytes: &[u8]) -> io::Result<PimdirObject> {
        let hash = self.blobs.hash(bytes);
        self.blobs.write(&hash, bytes)?;
        Ok(PimdirObject {
            hash,
            size: bytes.len(),
        })
    }

    /// A streaming writer for a new body: bytes go to a temporary file in
    /// objects/, hashed as they pass, and reach their content-addressed
    /// path on [`commit`](PimdirBlobWriter::commit).
    pub fn writer(&self) -> io::Result<PimdirBlobWriter> {
        let root = &self.blobs.root;
        modes::create_dir_all(root)?;
        let tmp = root.join(format!(".tmp-{}", unique()));
        let file = modes::options().write(true).create_new(true).open(&tmp)?;
        Ok(PimdirBlobWriter {
            blobs: self.blobs.clone(),
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
    blobs: PimdirBlobs,
    tmp: PathBuf,
    file: Option<fs::File>,
    hasher: Option<PimdirHasher>,
    written: u64,
    _lock: Arc<PimdirLock>,
}

impl PimdirBlobWriter {
    /// Finalises the body under the hash of the bytes written: `fsync`,
    /// then a move onto its path that never replaces a file (§5). A body
    /// already present keeps the stored copy. Answers the object it names.
    pub fn commit(mut self) -> io::Result<PimdirObject> {
        let file = self.file.take().expect("writer open");
        let synced = file.sync_all();
        drop(file);
        if let Err(err) = synced {
            let _ = fs::remove_file(&self.tmp);
            return Err(err);
        }

        let hash = self.hasher.take().expect("writer open").finish();
        let object = PimdirObject {
            hash,
            size: self.written as usize,
        };

        let blobs = &self.blobs;
        let (shard, name) = blobs.locate(&object.hash)?;
        let path = shard.join(name);
        let created = match blobs.shard(&shard) {
            Ok(created) => created,
            Err(err) => {
                let _ = fs::remove_file(&self.tmp);
                return Err(err);
            }
        };
        if fs::symlink_metadata(&path).is_ok() {
            let _ = fs::remove_file(&self.tmp);
            return Ok(object);
        }
        blobs.settle(&self.tmp, &shard, &path, created)?;

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

/// What sits where a body belongs, opened without following a link.
enum PimdirBlobOpen {
    /// A regular file, open.
    Body(fs::File),
    /// Nothing.
    Missing,
    /// A link, or anything else that is no regular file, at the body's
    /// shard or its own path: no body (§5).
    Stray(PathBuf),
}

/// What a check of a body against its name found (§7).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PimdirBlobCheck {
    /// It hashes to its name.
    Sound,
    /// Nothing sits where it belongs.
    Missing,
    /// What sits there is no body: bytes hashing to another name, or a
    /// link.
    Failing,
}

/// Whether a directory name is a shard's: two characters of the name
/// alphabet (§5).
fn is_shard(name: &str) -> bool {
    name.len() == 2 && name.bytes().all(object::is_name_byte)
}

/// Classifies an entry of objects/ that is no body.
fn stray(path: PathBuf, name: &str, kind: fs::FileType) -> PimdirBlobStray {
    let kind = if kind.is_symlink() {
        PimdirBlobStrayKind::Link
    } else if name.starts_with('.') && kind.is_file() {
        PimdirBlobStrayKind::Temporary
    } else {
        PimdirBlobStrayKind::Foreign
    };

    PimdirBlobStray { path, kind }
}

/// Hashes a whole file through its descriptor, from where it stands.
fn hash_file(file: &mut fs::File) -> io::Result<PimdirHash> {
    let mut hasher = PimdirHasher::new();
    let mut buffer = vec![0; CHUNK];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => hasher.update(&buffer[..read]),
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        }
    }

    Ok(hasher.finish())
}

/// A suffix unique to this process and this call: the process id, a
/// per-process counter and the time, so a temporary or a quarantined name
/// is never another writer's.
fn unique() -> String {
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();

    format!("{}-{seq}-{nanos}", process::id())
}

/// Opens `name` in `shard` following no link at either (§5): the shard
/// through `openat` with `O_DIRECTORY | O_NOFOLLOW`, then the body with
/// `O_NOFOLLOW` relative to it, so a link swapped in between is not
/// followed either.
#[cfg(unix)]
fn open_unfollowed(shard: &Path, name: &str) -> io::Result<fs::File> {
    use rustix::fs::{CWD, Mode, OFlags, openat};

    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let dir = openat(CWD, shard, flags | OFlags::DIRECTORY, Mode::empty())?;
    let file = openat(&dir, name, flags, Mode::empty())?;

    Ok(fs::File::from(file))
}

/// Opens `name` in `shard` following no link at either (§5): where no
/// link-free open exists, each is read by `lstat` before the open.
#[cfg(not(unix))]
fn open_unfollowed(shard: &Path, name: &str) -> io::Result<fs::File> {
    let path = shard.join(name);
    for entry in [shard, path.as_path()] {
        if fs::symlink_metadata(entry)?.file_type().is_symlink() {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                format!("{} is a symbolic link", entry.display()),
            ));
        }
    }

    fs::File::open(path)
}

/// Flushes a directory, so a name created or moved in it survives a power
/// loss: syncing a file makes its bytes durable and says nothing about the
/// name that reaches them (§5). On macOS std's `sync_all` is `F_FULLFSYNC`.
#[cfg(not(windows))]
fn sync_dir(dir: &Path) -> io::Result<()> {
    fs::File::open(dir)?.sync_all()
}

/// A directory cannot be flushed on Windows (§5): nothing to do.
#[cfg(windows)]
fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}
