//! # Hit columns
//!
//! A cache beside index.db, `index.hits`, holding what a count or a dated
//! page reads of each live placement as flat columns: its date, public id,
//! account, system flags and kind by index placement id, the placement of
//! each body by object id, and each hit's sort key. It is derived from the
//! index and the store alone, stamped with the store change and purges
//! the index folded and the index's highest placement and object ids, and
//! read only while the index holds the store as it stands and the stamp
//! is the index's own: any other answer reads index.db as before. A
//! refresh writes it again whenever its stamp differs, beside the file
//! then renamed over it, so a reader maps one whole file or none. The
//! file is never truncated in place, so a map stays that of its file.

#[cfg(feature = "indexer")]
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::{string::String, vec, vec::Vec};
#[cfg(feature = "indexer")]
use std::{
    collections::HashMap,
    io::{Read, Write},
};
use std::{
    fs::{self, File},
    path::Path,
};

#[cfg(feature = "indexer")]
use log::debug;
use rusqlite::Connection;
#[cfg(feature = "indexer")]
use rusqlite::OptionalExtension;

use crate::{
    client::{PimdirError, PreparedCached},
    search::plan::PimdirPlanNode,
    sql::search,
};

/// The cache's name in the store's directory.
pub(crate) const FILE: &str = "index.hits";

/// The cache's first word, its version and byte order.
const MAGIC: u64 = u64::from_ne_bytes(*b"PIMHITS3");

/// How many words the header holds.
const HEADER: usize = 16;

/// The header bit telling every dated sort key reads as a number.
const EXACT_DATES: u64 = 1;

/// The header bit telling every hit's dated mail placements share one
/// date.
const UNIFORM_DATES: u64 = 2;

/// The most account and kind names a cache holds, each a bit of a mask.
#[cfg(feature = "indexer")]
pub(crate) const NAMES_MAX: usize = 64;

/// The most words the per-hit sort keys may take.
#[cfg(feature = "indexer")]
const HIT_KEYS_MAX: u64 = 1 << 28;

/// A hit's sort key when one of its placements is of an undated kind.
pub(crate) const UNDATED: i64 = -1;

/// No hit at that public id.
pub(crate) const NO_HIT: i64 = i64::MIN;

/// The media types holding no date (SEARCH §8.2).
#[cfg(feature = "indexer")]
const UNDATED_KINDS: [&str; 2] = ["text/vcard", "application/octet-stream"];

/// What identifies the index state a cache was derived from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Stamp {
    store_change: i64,
    store_purges: i64,
    placement_max: i64,
    object_max: i64,
    /// index.db's inode, 0 where none is told: a rebuilt index is
    /// another file, whatever ids it reaches.
    file: u64,
}

impl Stamp {
    /// The stamp of the index `conn` holds as it stands in `dir`.
    pub(crate) fn of(conn: &Connection, dir: &Path) -> Result<Self, PimdirError> {
        let (store_change, store_purges, placement_max, object_max) = conn.query_row_cached(
            "SELECT store_change, store_purges, \
             (SELECT ifnull(max(id), 0) FROM placement), \
             (SELECT ifnull(max(id), 0) FROM object) \
             FROM index_meta WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        Ok(Self {
            store_change,
            store_purges,
            placement_max,
            object_max,
            file: inode(&dir.join("index.db")),
        })
    }

    fn words(&self) -> [u64; 4] {
        [
            self.store_change as u64,
            self.store_purges as u64,
            self.placement_max as u64,
            self.object_max as u64,
        ]
    }
}

/// The inode of the file at `path` and its birth time, 0 for what is not
/// told: a filesystem may hand a removed file's inode to the next one
/// created, a rebuilt index.db among them, but not its birth time.
fn inode(path: &Path) -> u64 {
    let Ok(metadata) = fs::metadata(path) else {
        return 0;
    };
    let born = metadata
        .created()
        .ok()
        .and_then(|born| born.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |born| born.as_nanos() as u64);
    #[cfg(unix)]
    let ino = {
        use std::os::unix::fs::MetadataExt;
        metadata.ino()
    };
    #[cfg(not(unix))]
    let ino: u64 = 0;
    ino.rotate_left(32) ^ born
}

/// The words of a cache, mapped or read.
enum Words {
    #[cfg(unix)]
    Mapped {
        ptr: *mut core::ffi::c_void,
        len: usize,
    },
    #[cfg_attr(unix, allow(dead_code))]
    Owned(Vec<u64>),
}

impl Words {
    fn open(path: &Path) -> Option<Self> {
        let file = File::open(path).ok()?;
        let len = file.metadata().ok()?.len() as usize;
        if len < HEADER * 8 || !len.is_multiple_of(8) {
            return None;
        }
        #[cfg(unix)]
        {
            // SAFETY: a read-only private map of a whole regular file.
            let ptr = unsafe {
                rustix::mm::mmap(
                    core::ptr::null_mut(),
                    len,
                    rustix::mm::ProtFlags::READ,
                    rustix::mm::MapFlags::PRIVATE,
                    &file,
                    0,
                )
            }
            .ok()?;
            Some(Self::Mapped { ptr, len })
        }
        #[cfg(not(unix))]
        {
            let mut file = file;
            let mut words = vec![0u64; len / 8];
            // SAFETY: the words are plain integers, any bytes are valid.
            let bytes =
                unsafe { core::slice::from_raw_parts_mut(words.as_mut_ptr().cast::<u8>(), len) };
            file.read_exact(bytes).ok()?;
            Some(Self::Owned(words))
        }
    }

    fn get(&self) -> &[u64] {
        match self {
            #[cfg(unix)]
            // SAFETY: page-aligned, `len` bytes, a multiple of 8, owned.
            Self::Mapped { ptr, len } => unsafe {
                core::slice::from_raw_parts(ptr.cast::<u64>(), len / 8)
            },
            Self::Owned(words) => words,
        }
    }
}

impl Drop for Words {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Self::Mapped { ptr, len } = self {
            // SAFETY: the map `open` made, unmapped once.
            let _ = unsafe { rustix::mm::munmap(*ptr, *len) };
        }
    }
}

/// A cache, mapped and checked.
pub(crate) struct PimdirHitColumns {
    words: Words,
    /// The account names by number, empty for none.
    pub(crate) accounts: Vec<String>,
    /// The collection kinds by number, from 1.
    pub(crate) kinds: Vec<String>,
    /// Whether every dated sort key reads as a number.
    pub(crate) exact_dates: bool,
    /// The highest public id.
    pub(crate) seq_max: usize,
    stamp: Stamp,
    records: (usize, usize),
    objects: (usize, usize),
    multi: (usize, usize),
    /// Where the index placement of each store item id starts, two to a
    /// word, and how many item ids it spans.
    items: (usize, usize),
    keys: usize,
    /// Where each hit's held word starts: the union of its placements'
    /// marks in its low byte, the date of its dated mail placements above.
    held: usize,
    /// Whether every hit's dated mail placements share one date.
    pub(crate) uniform_dates: bool,
}

/// One placement's record.
#[derive(Clone, Copy)]
pub(crate) struct Record {
    /// Its sort key as [`date_of`] reads it.
    pub(crate) date: i64,
    pub(crate) seq: usize,
    pub(crate) account: usize,
    pub(crate) marks: i64,
    /// The kind's number, from 1.
    pub(crate) kind: usize,
}

#[cfg(feature = "indexer")]
fn pack(seq: u64, account: u64, marks: u64, kind: u64) -> u64 {
    (seq & 0xffff_ffff) | (account & 0xffff) << 32 | (marks & 0xff) << 48 | (kind & 0xff) << 56
}

fn unpack(date: u64, packed: u64) -> Option<Record> {
    let kind = (packed >> 56) as usize;
    (kind != 0).then_some(Record {
        date: date as i64,
        seq: (packed & 0xffff_ffff) as usize,
        account: ((packed >> 32) & 0xffff) as usize,
        marks: ((packed >> 48) & 0xff) as i64,
        kind,
    })
}

fn names(words: &[u64], at: &mut usize, count: usize) -> Option<Vec<String>> {
    let mut names = Vec::with_capacity(count);
    for _ in 0..count {
        let len = *words.get(*at)? as usize;
        *at += 1;
        let need = len.div_ceil(8);
        let bytes: Vec<u8> = words
            .get(*at..*at + need)?
            .iter()
            .flat_map(|word| word.to_ne_bytes())
            .take(len)
            .collect();
        *at += need;
        names.push(String::from_utf8(bytes).ok()?);
    }
    Some(names)
}

#[cfg(feature = "indexer")]
fn put_names(out: &mut Vec<u64>, names: &[String]) {
    for name in names {
        out.push(name.len() as u64);
        for chunk in name.as_bytes().chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            out.push(u64::from_ne_bytes(word));
        }
    }
}

/// A sort key `YYYY-MM-DDTHH:MM:SSZ` as the number `YYYYMMDDHHMMSS`,
/// which orders as the key does; 0 for the empty key; `None` for another
/// shape.
pub(crate) fn date_of(key: &str) -> Option<i64> {
    if key.is_empty() {
        return Some(0);
    }
    let bytes = key.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return None;
    }
    let mut date = 0i64;
    for at in [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18] {
        let digit = bytes[at].wrapping_sub(b'0');
        if digit > 9 {
            return None;
        }
        date = date * 10 + i64::from(digit);
    }
    Some(date)
}

impl PimdirHitColumns {
    /// The cache of `dir` when its stamp is `stamp`.
    pub(crate) fn open(dir: &Path, stamp: Stamp) -> Option<Self> {
        let words = Words::open(&dir.join(FILE))?;
        let w = words.get();
        if w[0] != MAGIC || w[1..5] != stamp.words() || w[12] != stamp.file {
            return None;
        }
        let (records, objects, multi, seq_max) =
            (w[5] as usize, w[6] as usize, w[7] as usize, w[8] as usize);
        let (accounts, kinds, flags) = (w[9] as usize, w[10] as usize, w[11]);
        let items = w[13] as usize;
        let mut at = HEADER;
        let account_names = names(w, &mut at, accounts)?;
        let kind_names = names(w, &mut at, kinds)?;
        let records = (at, records);
        at += records.1 * 2;
        let objects = (at, objects);
        at += objects.1.div_ceil(2);
        let multi = (at, multi);
        at += multi.1;
        let items = (at, items);
        at += items.1.div_ceil(2);
        let keys = at;
        at += accounts * (seq_max + 1);
        let held = at;
        at += accounts * (seq_max + 1);
        if at != w.len() {
            return None;
        }
        Some(Self {
            words,
            accounts: account_names,
            kinds: kind_names,
            exact_dates: flags & EXACT_DATES != 0,
            uniform_dates: flags & UNIFORM_DATES != 0,
            held,
            seq_max,
            stamp,
            records,
            objects,
            multi,
            items,
            keys,
        })
    }

    /// Whether the cache was derived from the index state `stamp`.
    pub(crate) fn is(&self, stamp: Stamp) -> bool {
        self.stamp == stamp
    }

    /// How many placement ids the records span.
    pub(crate) fn placements(&self) -> usize {
        self.records.1
    }

    /// Calls `each` with every live placement's record.
    pub(crate) fn each(&self, mut each: impl FnMut(Record)) {
        let w = self.words.get();
        for pair in w[self.records.0..self.records.0 + self.records.1 * 2]
            .as_chunks::<2>()
            .0
        {
            if let Some(record) = unpack(pair[0], pair[1]) {
                each(record);
            }
        }
    }

    /// The record of index placement `id`, `None` for no live placement.
    pub(crate) fn record(&self, id: i64) -> Option<Record> {
        let id = usize::try_from(id).ok().filter(|id| *id < self.records.1)?;
        let w = self.words.get();
        let at = self.records.0 + id * 2;
        unpack(w[at], w[at + 1])
    }

    /// Calls `each` with the record of every placement of body `object`.
    pub(crate) fn of_object(&self, object: i64, mut each: impl FnMut(Record)) {
        let Some(object) = usize::try_from(object)
            .ok()
            .filter(|object| *object < self.objects.1)
        else {
            return;
        };
        let w = self.words.get();
        let word = w[self.objects.0 + object / 2];
        let placement = match object % 2 {
            0 => word & 0xffff_ffff,
            _ => word >> 32,
        };
        match placement {
            0 => {}
            0xffff_ffff => {
                let multi = &w[self.multi.0..self.multi.0 + self.multi.1];
                let first = multi.partition_point(|pair| (pair >> 32) < object as u64);
                for pair in &multi[first..] {
                    if (pair >> 32) != object as u64 {
                        break;
                    }
                    if let Some(record) = self.record((pair & 0xffff_ffff) as i64) {
                        each(record);
                    }
                }
            }
            placement => {
                if let Some(record) = self.record(placement as i64) {
                    each(record);
                }
            }
        }
    }

    /// The record of the placement of store item `item`, `None` for no
    /// live placement.
    pub(crate) fn of_item(&self, item: i64) -> Option<Record> {
        let item = usize::try_from(item)
            .ok()
            .filter(|item| *item < self.items.1)?;
        let word = self.words.get()[self.items.0 + item / 2];
        let placement = match item % 2 {
            0 => word & 0xffff_ffff,
            _ => word >> 32,
        };
        match placement {
            0 => None,
            placement => self.record(placement as i64),
        }
    }

    /// The sort key of hit `(account, seq)`: the greatest of its
    /// placements' dates, [`UNDATED`] when one is of an undated kind,
    /// [`NO_HIT`] for none.
    pub(crate) fn key(&self, account: usize, seq: usize) -> i64 {
        if seq > self.seq_max || account >= self.accounts.len() {
            return NO_HIT;
        }
        self.words.get()[self.keys + account * (self.seq_max + 1) + seq] as i64
    }

    /// The union of the system flags of hit `(account, seq)`'s
    /// placements, and the date of its dated mail placements (0 for none),
    /// the least where they differ.
    pub(crate) fn held(&self, account: usize, seq: usize) -> (i64, i64) {
        if seq > self.seq_max || account >= self.accounts.len() {
            return (0, 0);
        }
        let word = self.words.get()[self.held + account * (self.seq_max + 1) + seq];
        ((word & 0xff) as i64, (word >> 8) as i64)
    }

    /// Whether kind number `kind` is `message/rfc822`.
    pub(crate) fn is_mail(&self, kind: usize) -> bool {
        kind >= 1
            && self
                .kinds
                .get(kind - 1)
                .is_some_and(|name| name == "message/rfc822")
    }
}

/// Whether the cache of `dir` is stamped `stamp`, read from its header.
#[cfg(feature = "indexer")]
pub(crate) fn holds(dir: &Path, stamp: Stamp) -> bool {
    let Ok(mut file) = File::open(dir.join(FILE)) else {
        return false;
    };
    let mut head = [0u8; 104];
    if file.read_exact(&mut head).is_err() {
        return false;
    }
    let word = |at: usize| {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&head[at * 8..at * 8 + 8]);
        u64::from_ne_bytes(bytes)
    };
    word(0) == MAGIC
        && [word(1), word(2), word(3), word(4)] == stamp.words()
        && word(12) == stamp.file
}

/// The most placement keys a refresh follows for [`write`] to patch the
/// cache it found rather than read the index whole.
#[cfg(feature = "indexer")]
const TOUCHED_MAX: usize = 1 << 15;

/// What a refresh fed and wrote of the placements, so the cache it found
/// stamped as the index stood before it is patched where they changed.
#[derive(Default)]
#[cfg(feature = "indexer")]
pub(crate) struct Touched {
    /// The key of every placement fed or written.
    keys: BTreeSet<(String, i64)>,
    /// Every placement id put or dropped, with the body it named before
    /// the refresh, `None` for none or no placement.
    ids: BTreeMap<i64, Option<i64>>,
    /// Whether more than [`TOUCHED_MAX`] keys moved, the rest untold.
    over: bool,
}

#[cfg(feature = "indexer")]
impl Touched {
    /// Notes the placement `(collection, seq)` fed or written.
    pub(crate) fn key(&mut self, collection: &str, seq: i64) {
        if self.over {
            return;
        }
        if self.keys.len() >= TOUCHED_MAX {
            self.over = true;
            self.keys = BTreeSet::new();
            self.ids = BTreeMap::new();
            return;
        }
        self.keys.insert((collection.into(), seq));
    }

    /// Notes placement `id` naming body `object` before it is written,
    /// unless an earlier write of the refresh already told it.
    pub(crate) fn before(&mut self, id: i64, object: Option<i64>) {
        if !self.over {
            self.ids.entry(id).or_insert(object);
        }
    }
}

/// A cache's contents, before they are written.
#[cfg(feature = "indexer")]
struct Built {
    accounts: Vec<String>,
    kinds: Vec<String>,
    seq_max: usize,
    exact: bool,
    uniform: bool,
    records: Vec<u64>,
    objects: Vec<u32>,
    multi: Vec<u64>,
    items: Vec<u32>,
    keys: Vec<i64>,
    held: Vec<u64>,
}

/// Writes the cache of `dir` stamped `stamp`, unless it already is, beside
/// it then renamed in; removes it when the store is past what a cache
/// holds. With `since`, the stamp of the cache a refresh found and what it
/// fed and wrote since, the cache found is patched where they moved;
/// otherwise, or when the patch cannot tell, it is read from the index
/// `conn` holds with the store attached, whole.
#[cfg(feature = "indexer")]
pub(crate) fn write(
    conn: &Connection,
    dir: &Path,
    stamp: Stamp,
    since: Option<(Stamp, &Touched)>,
) -> Result<(), PimdirError> {
    if holds(dir, stamp) {
        return Ok(());
    }
    let path = dir.join(FILE);
    if let Some((old, touched)) = since
        && !touched.over
        && old.file == stamp.file
        && let Some(columns) = PimdirHitColumns::open(dir, old)
        && let Some(built) = patch(conn, &columns, stamp, touched)?
    {
        drop(columns);
        debug!(
            "hit columns patched where {} placement keys moved",
            touched.keys.len()
        );
        return emit(&path, stamp, &built);
    }

    let gone = |path: &Path| {
        let _ = fs::remove_file(path);
        Ok(())
    };
    let seq_max: i64 = conn.query_row("SELECT ifnull(max(seq), 0) FROM placement", [], |row| {
        row.get(0)
    })?;
    let item_max: i64 =
        conn.query_row("SELECT ifnull(max(id), 0) FROM store.items", [], |row| {
            row.get(0)
        })?;
    if stamp.placement_max >= 0xffff_ffff
        || !(0..0xffff_ffff).contains(&item_max)
        || stamp.object_max >= 0xffff_ffff
        || !(0..0xffff_ffff).contains(&seq_max)
    {
        return gone(&path);
    }

    let mut accounts: Vec<String> = Vec::new();
    let mut kinds: Vec<String> = Vec::new();
    let mut collections: HashMap<String, (usize, usize)> = HashMap::new();
    {
        let mut statement =
            conn.prepare("SELECT id, ifnull(account, ''), kind FROM store.collections")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let (id, account, kind): (String, String, String) =
                (row.get(0)?, row.get(1)?, row.get(2)?);
            let account = match accounts.iter().position(|known| *known == account) {
                Some(at) => at,
                None => {
                    accounts.push(account);
                    accounts.len() - 1
                }
            };
            let kind = match kinds.iter().position(|known| *known == kind) {
                Some(at) => at + 1,
                None => {
                    kinds.push(kind);
                    kinds.len()
                }
            };
            collections.insert(id, (account, kind));
        }
    }
    let keys_len = accounts.len() as u64 * (seq_max as u64 + 1);
    if accounts.len() > NAMES_MAX || kinds.len() >= NAMES_MAX || keys_len > HIT_KEYS_MAX {
        return gone(&path);
    }
    let undated: Vec<bool> = core::iter::once(false)
        .chain(
            kinds
                .iter()
                .map(|kind| UNDATED_KINDS.contains(&kind.as_str())),
        )
        .collect();

    let records_len = stamp.placement_max as usize + 1;
    let objects_len = stamp.object_max as usize + 1;
    let row_len = seq_max as usize + 1;
    let mut records = vec![0u64; records_len * 2];
    let mut objects = vec![0u32; objects_len];
    let mut multi: Vec<u64> = Vec::new();
    let items_len = item_max as usize + 1;
    let mut items = vec![0u32; items_len];
    let mut keys = vec![NO_HIT; keys_len as usize];
    let mut held = vec![0u64; keys_len as usize];
    let mut uniform = true;
    let mail: Vec<bool> = core::iter::once(false)
        .chain(kinds.iter().map(|kind| kind == "message/rfc822"))
        .collect();
    let mut exact = true;
    {
        let mut statement = conn.prepare(
            "SELECT p.id, p.object, p.collection, p.seq, i.sort_key, i.marks, i.id \
             FROM placement p JOIN store.items i \
             ON i.collection = p.collection AND i.seq = p.seq AND i.deleted = 0",
        )?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let object: Option<i64> = row.get(1)?;
            let collection = row.get_ref(2)?.as_str().map_err(rusqlite::Error::from)?;
            let seq: i64 = row.get(3)?;
            let key = row.get_ref(4)?.as_str().map_err(rusqlite::Error::from)?;
            let marks: i64 = row.get(5)?;
            let item: i64 = row.get(6)?;
            let Some(&(account, kind)) = collections.get(collection) else {
                continue;
            };
            if !(1..records_len as i64).contains(&id)
                || !(0..=seq_max).contains(&seq)
                || !(0..items_len as i64).contains(&item)
            {
                return gone(&path);
            }
            items[item as usize] = id as u32;
            let date = date_of(key).unwrap_or_else(|| {
                exact = false;
                0
            });
            let (id, seq) = (id as usize, seq as usize);
            records[id * 2] = date as u64;
            records[id * 2 + 1] = pack(seq as u64, account as u64, marks as u64, kind as u64);
            if let Some(object) = object.and_then(|object| usize::try_from(object).ok()) {
                if object >= objects_len {
                    return gone(&path);
                }
                objects[object] = match objects[object] {
                    0 => id as u32,
                    first => {
                        if first != u32::MAX {
                            multi.push((object as u64) << 32 | u64::from(first));
                        }
                        multi.push((object as u64) << 32 | id as u64);
                        u32::MAX
                    }
                };
            }
            let word = &mut held[account * row_len + seq];
            *word |= marks as u64 & 0xff;
            if mail[kind] && date > 0 {
                let was = (*word >> 8) as i64;
                if was == 0 || date < was {
                    uniform &= was == 0;
                    *word = (*word & 0xff) | (date as u64) << 8;
                } else if date != was {
                    uniform = false;
                }
            }
            let slot = &mut keys[account * row_len + seq];
            *slot = match (undated[kind], *slot) {
                (true, _) | (_, UNDATED) => UNDATED,
                (false, NO_HIT) => date,
                (false, held) => held.max(date),
            };
        }
    }
    multi.sort_unstable();

    let built = Built {
        accounts,
        kinds,
        seq_max: seq_max as usize,
        exact,
        uniform,
        records,
        objects,
        multi,
        items,
        keys,
        held,
    };
    emit(&path, stamp, &built)
}

/// Writes `built` stamped `stamp` beside `path`, then renames it over.
#[cfg(feature = "indexer")]
fn emit(path: &Path, stamp: Stamp, built: &Built) -> Result<(), PimdirError> {
    let mut head: Vec<u64> = vec![0; HEADER];
    head[0] = MAGIC;
    head[1..5].copy_from_slice(&stamp.words());
    head[12] = stamp.file;
    head[13] = built.items.len() as u64;
    head[5] = (built.records.len() / 2) as u64;
    head[6] = built.objects.len() as u64;
    head[7] = built.multi.len() as u64;
    head[8] = built.seq_max as u64;
    head[9] = built.accounts.len() as u64;
    head[10] = built.kinds.len() as u64;
    head[11] =
        if built.exact { EXACT_DATES } else { 0 } | if built.uniform { UNIFORM_DATES } else { 0 };
    put_names(&mut head, &built.accounts);
    put_names(&mut head, &built.kinds);

    /// Writes words as their bytes.
    fn words(file: &mut File, words: &[u64]) -> std::io::Result<()> {
        // SAFETY: the words are plain integers viewed as their bytes.
        let bytes =
            unsafe { core::slice::from_raw_parts(words.as_ptr().cast::<u8>(), words.len() * 8) };
        file.write_all(bytes)
    }
    /// Writes ids two to a word, the first in the low half.
    fn halves(file: &mut File, ids: &[u32]) -> std::io::Result<()> {
        let mut buffer = Vec::with_capacity(ids.len().div_ceil(2).min(1 << 16));
        for chunk in ids.chunks(1 << 17) {
            buffer.clear();
            buffer.extend(
                chunk
                    .chunks(2)
                    .map(|pair| u64::from(pair[0]) | u64::from(*pair.get(1).unwrap_or(&0)) << 32),
            );
            words(file, &buffer)?;
        }
        Ok(())
    }

    let new = path.with_file_name(".index.hits.new");
    {
        let mut file = File::create(&new)?;
        words(&mut file, &head)?;
        words(&mut file, &built.records)?;
        halves(&mut file, &built.objects)?;
        words(&mut file, &built.multi)?;
        halves(&mut file, &built.items)?;
        // SAFETY: an i64 has the size and alignment of a u64, any bits valid.
        let keys = unsafe {
            core::slice::from_raw_parts(built.keys.as_ptr().cast::<u64>(), built.keys.len())
        };
        words(&mut file, keys)?;
        words(&mut file, &built.held)?;
    }
    fs::rename(&new, path)?;
    Ok(())
}

/// The cache `old` patched to the index `conn` holds as stamped `stamp`,
/// where `touched` tells what moved since: each placement fed or written
/// read again, with the bodies and hits it named before and names now;
/// `None` when the patch cannot tell (a collection's account or kind
/// moved, an id range shrank past a live entry, a bound a cache holds
/// passed), for the whole index to be read.
#[cfg(feature = "indexer")]
fn patch(
    conn: &Connection,
    old: &PimdirHitColumns,
    stamp: Stamp,
    touched: &Touched,
) -> Result<Option<Built>, PimdirError> {
    let w = old.words.get();
    let seq_now: i64 =
        conn.query_row_cached("SELECT ifnull(max(seq), 0) FROM placement", [], |row| {
            row.get(0)
        })?;
    let item_max: i64 =
        conn.query_row_cached("SELECT ifnull(max(id), 0) FROM store.items", [], |row| {
            row.get(0)
        })?;
    if stamp.placement_max >= 0xffff_ffff
        || !(0..0xffff_ffff).contains(&item_max)
        || stamp.object_max >= 0xffff_ffff
        || !(0..0xffff_ffff).contains(&seq_now)
    {
        return Ok(None);
    }
    let records_len = stamp.placement_max as usize + 1;
    let objects_len = stamp.object_max as usize + 1;
    let items_len = item_max as usize + 1;
    let records_cap = records_len.max(old.records.1);
    let objects_cap = objects_len.max(old.objects.1);
    let items_cap = items_len.max(old.items.1);
    let seq_max = old.seq_max.max(seq_now as usize);
    let mut accounts = old.accounts.clone();
    let mut kinds = old.kinds.clone();

    let mut ids: BTreeSet<i64> = touched.ids.keys().copied().collect();
    for (collection, seq) in &touched.keys {
        let id: Option<i64> = conn
            .query_row_cached(
                "SELECT id FROM placement WHERE collection = ?1 AND seq = ?2",
                rusqlite::params![collection, seq],
                |row| row.get(0),
            )
            .optional()?;
        ids.extend(id);
    }

    {
        let mut changed = conn.prepare_cached(
            "SELECT id, ifnull(account, ''), kind FROM store.collections WHERE changed > ?1",
        )?;
        let mut rows = changed.query([old.stamp.store_change])?;
        while let Some(row) = rows.next()? {
            let collection = row.get_ref(0)?.as_str().map_err(rusqlite::Error::from)?;
            let account = row.get_ref(1)?.as_str().map_err(rusqlite::Error::from)?;
            let kind = row.get_ref(2)?.as_str().map_err(rusqlite::Error::from)?;
            let mut placements =
                conn.prepare_cached("SELECT id FROM placement WHERE collection = ?1")?;
            let mut placed = placements.query([collection])?;
            while let Some(placement) = placed.next()? {
                let id: i64 = placement.get(0)?;
                if touched.ids.contains_key(&id) {
                    continue;
                }
                if let Some(record) = old.record(id) {
                    if accounts[record.account] != account || kinds[record.kind - 1] != kind {
                        return Ok(None);
                    }
                    break;
                }
            }
        }
    }

    let mut records = vec![0u64; records_cap * 2];
    records[..old.records.1 * 2]
        .copy_from_slice(&w[old.records.0..old.records.0 + old.records.1 * 2]);
    let unhalve = |(at, len): (usize, usize), to: usize| {
        let mut ids = vec![0u32; to];
        for (pair, word) in ids[..len].chunks_mut(2).zip(&w[at..at + len.div_ceil(2)]) {
            pair[0] = *word as u32;
            if let Some(second) = pair.get_mut(1) {
                *second = (*word >> 32) as u32;
            }
        }
        ids
    };
    let mut objects = unhalve(old.objects, objects_cap);
    let mut items = unhalve(old.items, items_cap);
    let old_multi = &w[old.multi.0..old.multi.0 + old.multi.1];

    let mut collections: HashMap<String, Option<(usize, usize)>> = HashMap::new();
    let mut attrs = |collection: &str| -> Result<Option<(usize, usize)>, PimdirError> {
        if let Some(known) = collections.get(collection) {
            return Ok(*known);
        }
        let row: Option<(String, String)> = conn
            .query_row_cached(
                "SELECT ifnull(account, ''), kind FROM store.collections WHERE id = ?1",
                [collection],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let known = row.map(|(account, kind)| {
            let account = match accounts.iter().position(|known| *known == account) {
                Some(at) => at,
                None => {
                    accounts.push(account);
                    accounts.len() - 1
                }
            };
            let kind = match kinds.iter().position(|known| *known == kind) {
                Some(at) => at + 1,
                None => {
                    kinds.push(kind);
                    kinds.len()
                }
            };
            (account, kind)
        });
        collections.insert(collection.into(), known);
        Ok(known)
    };

    if ids.iter().any(|id| !(1..records_cap as i64).contains(id)) {
        return Ok(None);
    }
    let mut read_again = vec![0u64; records_cap.div_ceil(64)];
    for &id in &ids {
        read_again[id as usize / 64] |= 1 << (id % 64);
    }
    for slot in &mut items {
        let at = *slot as usize;
        if read_again[at / 64] & 1 << (at % 64) != 0 {
            *slot = 0;
        }
    }

    let mut exact = old.exact_dates;
    let mut hits: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut dropped: Vec<(u64, u32)> = Vec::new();
    let mut added: Vec<(u64, u32)> = Vec::new();
    for &id in &ids {
        let current: Option<Option<i64>> = conn
            .query_row_cached("SELECT object FROM placement WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?;
        if let Some(record) = old.record(id) {
            hits.insert((record.account, record.seq));
            let object = match touched.ids.get(&id) {
                Some(object) => *object,
                None => current.flatten(),
            };
            if let Some(object) = object.and_then(|object| u64::try_from(object).ok()) {
                dropped.push((object, id as u32));
            }
        }
        let at = id as usize * 2;
        records[at] = 0;
        records[at + 1] = 0;

        let mut statement = conn.prepare_cached(
            "SELECT p.object, p.collection, p.seq, i.sort_key, i.marks, i.id \
             FROM placement p JOIN store.items i \
             ON i.collection = p.collection AND i.seq = p.seq AND i.deleted = 0 \
             WHERE p.id = ?1",
        )?;
        let mut rows = statement.query([id])?;
        while let Some(row) = rows.next()? {
            let object: Option<i64> = row.get(0)?;
            let collection = row.get_ref(1)?.as_str().map_err(rusqlite::Error::from)?;
            let seq: i64 = row.get(2)?;
            let key = row.get_ref(3)?.as_str().map_err(rusqlite::Error::from)?;
            let marks: i64 = row.get(4)?;
            let item: i64 = row.get(5)?;
            let Some((account, kind)) = attrs(collection)? else {
                continue;
            };
            if !(0..=seq_now).contains(&seq) || !(0..items_len as i64).contains(&item) {
                return Ok(None);
            }
            items[item as usize] = id as u32;
            let date = date_of(key).unwrap_or_else(|| {
                exact = false;
                0
            });
            records[at] = date as u64;
            records[at + 1] = pack(seq as u64, account as u64, marks as u64, kind as u64);
            hits.insert((account, seq as usize));
            if let Some(object) = object.and_then(|object| u64::try_from(object).ok()) {
                if object as usize >= objects_len {
                    return Ok(None);
                }
                added.push((object, id as u32));
            }
        }
    }
    let keys_len = accounts.len() as u64 * (seq_max as u64 + 1);
    if accounts.len() > NAMES_MAX || kinds.len() >= NAMES_MAX || keys_len > HIT_KEYS_MAX {
        return Ok(None);
    }

    let mut moved: BTreeMap<u64, Vec<u32>> = BTreeMap::new();
    for &(object, _) in dropped.iter().chain(&added) {
        moved.entry(object).or_insert_with(|| {
            let object_at = object as usize;
            match objects.get(object_at).copied().unwrap_or(0) {
                0 => Vec::new(),
                u32::MAX => {
                    let first = old_multi.partition_point(|pair| (pair >> 32) < object);
                    old_multi[first..]
                        .iter()
                        .take_while(|pair| (*pair >> 32) == object)
                        .map(|pair| *pair as u32)
                        .collect()
                }
                placement => vec![placement],
            }
        });
    }
    for (object, id) in dropped {
        if let Some(placements) = moved.get_mut(&object) {
            placements.retain(|placement| *placement != id);
        }
    }
    for (object, id) in added {
        if let Some(placements) = moved.get_mut(&object)
            && !placements.contains(&id)
        {
            placements.push(id);
        }
    }
    let mut multi: Vec<u64> = old_multi
        .iter()
        .copied()
        .filter(|pair| !moved.contains_key(&(pair >> 32)))
        .collect();
    for (object, placements) in &moved {
        let slot = &mut objects[*object as usize];
        *slot = match placements.as_slice() {
            [] => 0,
            [single] => *single,
            _ => {
                multi.extend(placements.iter().map(|id| object << 32 | u64::from(*id)));
                u32::MAX
            }
        };
    }
    multi.sort_unstable();
    if records[records_len * 2..].iter().any(|word| *word != 0)
        || objects[objects_len..].iter().any(|id| *id != 0)
        || items[items_len..].iter().any(|id| *id != 0)
    {
        return Ok(None);
    }
    records.truncate(records_len * 2);
    objects.truncate(objects_len);
    items.truncate(items_len);

    let row_len = seq_max + 1;
    let old_row = old.seq_max + 1;
    let mut keys = vec![NO_HIT; keys_len as usize];
    let mut held = vec![0u64; keys_len as usize];
    for account in 0..old.accounts.len() {
        let from = account * old_row;
        let to = account * row_len;
        for (slot, word) in keys[to..to + old_row]
            .iter_mut()
            .zip(&w[old.keys + from..old.keys + from + old_row])
        {
            *slot = *word as i64;
        }
        held[to..to + old_row].copy_from_slice(&w[old.held + from..old.held + from + old_row]);
    }
    let undated: Vec<bool> = core::iter::once(false)
        .chain(
            kinds
                .iter()
                .map(|kind| UNDATED_KINDS.contains(&kind.as_str())),
        )
        .collect();
    let mail: Vec<bool> = core::iter::once(false)
        .chain(kinds.iter().map(|kind| kind == "message/rfc822"))
        .collect();
    let mut uniform = old.uniform_dates;
    let mut by_seq = conn.prepare_cached("SELECT id FROM placement WHERE seq = ?1")?;
    for (account, seq) in hits {
        let mut key = NO_HIT;
        let mut word = 0u64;
        let mut rows = by_seq.query([seq as i64])?;
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let Some(at) = usize::try_from(id).ok().filter(|at| *at < records_len) else {
                continue;
            };
            let Some(record) = unpack(records[at * 2], records[at * 2 + 1]) else {
                continue;
            };
            if record.account != account || record.seq != seq {
                continue;
            }
            let date = record.date;
            word |= record.marks as u64 & 0xff;
            if mail[record.kind] && date > 0 {
                let was = (word >> 8) as i64;
                if was == 0 || date < was {
                    uniform &= was == 0;
                    word = (word & 0xff) | (date as u64) << 8;
                } else if date != was {
                    uniform = false;
                }
            }
            key = match (undated[record.kind], key) {
                (true, _) | (_, UNDATED) => UNDATED,
                (false, NO_HIT) => date,
                (false, held) => held.max(date),
            };
        }
        keys[account * row_len + seq] = key;
        held[account * row_len + seq] = word;
    }

    let seq_now = seq_now as usize;
    if seq_now < seq_max {
        let clean = keys
            .chunks(row_len)
            .all(|row| row[seq_now + 1..].iter().all(|key| *key == NO_HIT))
            && held
                .chunks(row_len)
                .all(|row| row[seq_now + 1..].iter().all(|word| *word == 0));
        if !clean {
            return Ok(None);
        }
        keys = keys
            .chunks(row_len)
            .flat_map(|row| &row[..=seq_now])
            .copied()
            .collect();
        held = held
            .chunks(row_len)
            .flat_map(|row| &row[..=seq_now])
            .copied()
            .collect();
    }
    let seq_max = seq_max.min(seq_now);

    Ok(Some(Built {
        accounts,
        kinds,
        seq_max,
        exact,
        uniform,
        records,
        objects,
        multi,
        items,
        keys,
        held,
    }))
}

/// How a leaf reads off the columns.
enum Column<'a> {
    /// A term the scope implies: every hit of the scope.
    Every,
    /// A system flag, its bit of `items.marks`.
    Mark(i64),
    /// A text term: its FTS5 expression, and whether a summary holds its
    /// field.
    Text(&'a str, bool),
    /// A date term: `[since, until)` as [`date_of`] reads a key.
    Date(i64, i64),
    /// An address term: the address lowercased, and its role, `None` for
    /// any.
    Address(String, Option<&'static str>),
}

impl super::PimdirIndex {
    /// The hits of `compiled` read off the hit columns, with each
    /// account's name by its number and the hits a positive text term
    /// was read off one of their bodies for ([`Run::proven`]): `None` when
    /// the index does not hold the store as it stands, no cache holds its
    /// state, or a term or the scope is one the columns cannot answer (a
    /// collection scope, a term other than system flags, text and dates
    /// over mail alone).
    #[allow(clippy::type_complexity)]
    pub(super) fn columnar(
        &self,
        compiled: &super::Compiled,
    ) -> Result<Option<(super::Hits, Vec<String>, super::Hits)>, PimdirError> {
        let plan = &compiled.plan;
        if self.force.get().is_some() || plan.scope.collections.is_some() {
            return Ok(None);
        }
        let current = self
            .conn
            .query_row_cached(crate::sql::search::INDEX_CURRENT, [], |row| {
                row.get::<_, bool>(0)
            })?;
        if !current {
            return Ok(None);
        }
        let stamp = Stamp::of(&self.conn, &self.dir)?;
        let mut held = self.columns.borrow_mut();
        if !held.as_ref().is_some_and(|columns| columns.is(stamp)) {
            *held = PimdirHitColumns::open(&self.dir, stamp);
        }
        let Some(columns) = held.as_ref() else {
            return Ok(None);
        };

        let dated = columns.exact_dates
            && plan.undated.is_empty()
            && !columns.kinds.iter().any(|kind| kind == "text/calendar");
        let mut read = Vec::with_capacity(plan.leaves.len());
        for (at, leaf) in plan.leaves.iter().enumerate() {
            use crate::search::query::PimdirQueryPredicate as P;
            let column = match &leaf.predicate {
                _ if plan.scoped.contains(&at) => Column::Every,
                P::Flag(flag) => match crate::codec::mark_of(flag) {
                    Some(bit) => Column::Mark(bit),
                    None => return Ok(None),
                },
                P::Date(_) if dated => {
                    let Some((since, until)) = leaf.dates else {
                        return Ok(None);
                    };
                    let bound = |at: Option<i64>, open: i64| match at {
                        None => Some(open),
                        Some(at) => date_of(&crate::summary::time::rfc3339(at)),
                    };
                    match (bound(since, i64::MIN), bound(until, i64::MAX)) {
                        (Some(since), Some(until)) => Column::Date(since, until),
                        _ => return Ok(None),
                    }
                }
                P::Address { role, text } if text.text.contains('@') => {
                    if text.text.starts_with('@') {
                        return Ok(None);
                    }
                    let role = role.map(|role| role.as_str());
                    Column::Address(text.text.to_lowercase(), role)
                }
                _ if leaf.is_text() => match compiled.text_of(at) {
                    Some((expression, summary)) => Column::Text(expression, summary),
                    None => return Ok(None),
                },
                _ => return Ok(None),
            };
            read.push(column);
        }

        let kinds: u64 = match &plan.scope.kinds {
            None => !0,
            Some(kinds) => columns
                .kinds
                .iter()
                .enumerate()
                .filter(|(_, kind)| kinds.contains(&kind.as_str()))
                .fold(0, |mask, (at, _)| mask | 1 << (at + 1)),
        };
        let accounts: Vec<bool> = match &plan.scope.accounts {
            None => vec![true; columns.accounts.len()],
            Some(accounts) => columns
                .accounts
                .iter()
                .map(|account| accounts.contains(account))
                .collect(),
        };
        let split: i64 = self.conn.query_row_cached(
            crate::sql::search::SPLIT_HELD,
            rusqlite::named_params! { ":limit": 1 },
            |row| row.get(0),
        )?;
        let mut run = Run {
            index: self,
            compiled,
            columns,
            read: &read,
            kinds,
            accounts: &accounts,
            words: columns.seq_max / 64 + 1,
            every: None,
            scanned: vec![None; read.len()],
            joint: split == 0,
            negations: 0,
            proven: Vec::new(),
        };
        let hits = run.eval(&plan.root)?;
        Ok(Some((hits, columns.accounts.clone(), run.proven)))
    }

    /// Marks each of `found` whose hit `proven` holds as matched in a
    /// body ([`Found::body`](super::Found)), `names` numbering the
    /// accounts as the columns do.
    fn prove(&self, found: &mut [super::Found], names: &[String], proven: &super::Hits) {
        for hit in found {
            let name = hit.account.as_deref().unwrap_or("");
            hit.body = names
                .iter()
                .position(|known| known == name)
                .zip(usize::try_from(hit.seq).ok())
                .and_then(|(account, seq)| proven.get(account)?.get(seq / 64))
                .is_some_and(|word| word >> (hit.seq % 64) & 1 == 1);
        }
    }

    /// Whether the store holds a calendar collection, read off the hit
    /// columns last mapped: their kinds are every collection's, read
    /// after the index folded the store change their stamp holds, so while
    /// the index holds the store as it stands and that stamp is the
    /// index's, no collection was made, retyped or removed since. `None`
    /// when no columns are mapped or they no longer answer.
    pub(super) fn columnar_calendars(&self) -> Result<Option<bool>, PimdirError> {
        let held = self.columns.borrow();
        let Some(columns) = held.as_ref() else {
            return Ok(None);
        };
        let current = self
            .conn
            .query_row_cached(crate::sql::search::INDEX_CURRENT, [], |row| {
                row.get::<_, bool>(0)
            })?;
        if !current || !columns.is(Stamp::of(&self.conn, &self.dir)?) {
            return Ok(None);
        }
        Ok(Some(
            columns.kinds.iter().any(|kind| kind == "text/calendar"),
        ))
    }

    /// The first `need` hits after `after` of a dated `sort` read off the
    /// hit columns ([`columnar_batch`](Self::columnar_batch)).
    pub(super) fn columnar_page(
        &self,
        compiled: &super::Compiled,
        after: Option<&super::PimdirSearchCursor>,
        sort: super::PimdirSearchSort,
        need: usize,
    ) -> Result<Option<Vec<super::Found>>, PimdirError> {
        if !sort.is_dated() || need == 0 {
            return Ok(None);
        }
        let Some((hits, names, proven)) = self.columnar(compiled)? else {
            return Ok(None);
        };
        self.columnar_batch(compiled, &hits, &names, &proven, after, sort, need)
    }

    /// The first `need` of `hits` after `after` in a dated `sort`: the
    /// hits whose sort key reaches the `need`-th past the cursor, those
    /// on the cursor's key counted in, written to `temp.hit_page`, then
    /// joined to the store, sorted and cut by one statement; the undated
    /// join them when fewer dated hits remain. `None` after an undated
    /// cursor, or past [`PAGE_HITS_MAX`](super::PAGE_HITS_MAX) of them.
    /// Each is marked matched in a body when `proven` holds it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn columnar_batch(
        &self,
        compiled: &super::Compiled,
        hits: &super::Hits,
        names: &[String],
        proven: &super::Hits,
        after: Option<&super::PimdirSearchCursor>,
        sort: super::PimdirSearchSort,
        need: usize,
    ) -> Result<Option<Vec<super::Found>>, PimdirError> {
        if !sort.is_dated() || need == 0 {
            return Ok(None);
        }
        let newest = sort == super::PimdirSearchSort::NewestFirst;
        let bound = match after {
            None => None,
            Some(after) if after.undated => return Ok(None),
            Some(after) => match date_of(after.sort_key.as_deref().unwrap_or("")) {
                Some(date) => Some(date),
                None => return Ok(None),
            },
        };
        let eligible = |key: i64| match bound {
            None => true,
            Some(bound) if newest => key <= bound,
            Some(bound) => key >= bound,
        };
        let held = self.columns.borrow();
        let Some(columns) = held.as_ref() else {
            return Ok(None);
        };
        let each = |each: &mut dyn FnMut(usize, usize, i64)| {
            for (account, bits) in hits.iter().enumerate() {
                for (at, word) in bits.iter().enumerate() {
                    let mut word = *word;
                    while word != 0 {
                        let seq = at * 64 + word.trailing_zeros() as usize;
                        each(account, seq, columns.key(account, seq));
                        word &= word - 1;
                    }
                }
            }
        };
        let mut dated: Vec<i64> = Vec::new();
        let mut undated = 0usize;
        let mut lost = false;
        each(&mut |_, _, key| match key {
            NO_HIT => lost = true,
            UNDATED => undated += 1,
            key if eligible(key) => dated.push(key),
            _ => {}
        });
        if lost {
            return Ok(None);
        }
        let want = need + dated.iter().filter(|key| Some(**key) == bound).count();
        let threshold = match dated.len() > want {
            false => None,
            true if newest => Some(*dated.select_nth_unstable_by(want - 1, |a, b| b.cmp(a)).1),
            true => Some(*dated.select_nth_unstable(want - 1).1),
        };
        let with_undated = dated.len() < want;
        let candidates = dated.len().min(want) + if with_undated { undated } else { 0 };
        if candidates as u64 > super::PAGE_HITS_MAX {
            return Ok(None);
        }

        let mut page = String::from("[");
        each(&mut |account, seq, key| {
            let wanted = match (key, threshold) {
                (UNDATED, _) => with_undated,
                (key, _) if !eligible(key) => false,
                (_, None) => true,
                (key, Some(threshold)) if newest => key >= threshold,
                (key, Some(threshold)) => key <= threshold,
            };
            if wanted {
                let name = names.get(account).map(String::as_str).unwrap_or("");
                if page.len() > 1 {
                    page.push(',');
                }
                page.push('[');
                json_string(&mut page, name);
                page.push_str(&alloc::format!(",{seq}]"));
            }
        });
        page.push(']');

        let sql = compiled.met_rows(
            "SELECT p.value ->> 0 AS akey, p.value ->> 1 AS seq FROM json_each(:page) p",
            sort,
            Some(after.is_some()),
        );
        let mut extra = super::page_params(after, need);
        extra.push((String::from(":page"), rusqlite::types::Value::from(page)));
        let mut found = self.run(&sql, &compiled.values, &extra, |row| {
            super::found(row, sort)
        })?;
        self.prove(&mut found, names, proven);
        Ok(Some(found))
    }

    /// The members of a thread, its `seqs`, among `hits` in the account of
    /// `hit`, the hit that met it, in `sort` order, as
    /// [`members`](super::PimdirIndex::members) reads them: `hit` itself
    /// when it is the only one, no statement run for it. Each is marked
    /// matched in a body when `proven` holds it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn columnar_members(
        &self,
        compiled: &super::Compiled,
        hits: &super::Hits,
        names: &[String],
        proven: &super::Hits,
        seqs: &alloc::collections::BTreeSet<i64>,
        hit: &super::Found,
        sort: super::PimdirSearchSort,
    ) -> Result<Vec<super::Found>, PimdirError> {
        let account = hit.account.as_deref();
        let name = account.unwrap_or_default();
        let Some(number) = names.iter().position(|known| known == name) else {
            return Ok(Vec::new());
        };
        let bits = &hits[number];
        let seqs: alloc::collections::BTreeSet<i64> = seqs
            .iter()
            .copied()
            .filter(|seq| {
                usize::try_from(*seq).is_ok_and(|seq| {
                    bits.get(seq / 64)
                        .is_some_and(|word| word >> (seq % 64) & 1 == 1)
                })
            })
            .collect();
        if seqs.is_empty() {
            return Ok(Vec::new());
        }
        if seqs.len() == 1 && seqs.contains(&hit.seq) {
            return Ok(vec![hit.clone()]);
        }
        let listed: Vec<String> = seqs.iter().map(|seq| alloc::format!("{seq}")).collect();
        let extra = vec![
            (
                String::from(":akey"),
                rusqlite::types::Value::from(String::from(name)),
            ),
            (
                String::from(":members"),
                rusqlite::types::Value::from(alloc::format!("[{}]", listed.join(","))),
            ),
        ];
        let members = "SELECT :akey AS akey, value AS seq FROM json_each(:members)";
        let sql = compiled.met_rows(members, sort, None);
        let mut found = self.run(&sql, &compiled.values, &extra, |row| {
            super::found(row, sort)
        })?;
        self.prove(&mut found, names, proven);
        found.sort_by(|left, right| left.place.compare(&right.place, sort));
        Ok(found)
    }
}

/// One evaluation of a query off the hit columns: every hit of the scope
/// and the scanned leaves read in one pass over the records when first
/// needed, text read off the full-text index alone.
struct Run<'a> {
    index: &'a super::PimdirIndex,
    compiled: &'a super::Compiled,
    columns: &'a PimdirHitColumns,
    read: &'a [Column<'a>],
    /// The scope's kinds, a bit per kind number.
    kinds: u64,
    /// Whether each account lies in the scope.
    accounts: &'a [bool],
    /// How many words a bitmap holds per account.
    words: usize,
    every: Option<super::Hits>,
    /// The hits of each flag and date leaf, once scanned.
    scanned: Vec<Option<super::Hits>>,
    /// Whether no hit holds several documents (`split` empty), so an `and`
    /// of text terms matches as one FTS5 expression.
    joint: bool,
    /// How many negations the node evaluated stands under.
    negations: usize,
    /// The hits a body of theirs matched a text term standing under no
    /// negation for, a positive term the page probes, so their `matched`
    /// is `body` with no probe; empty until one is read.
    proven: super::Hits,
}

impl Run<'_> {
    fn empty(&self) -> super::Hits {
        vec![vec![0u64; self.words]; self.columns.accounts.len()]
    }

    fn in_scope(&self, record: &Record) -> bool {
        self.kinds >> record.kind & 1 == 1 && self.accounts[record.account]
    }

    /// Reads every hit of the scope and each flag and date leaf's hits in
    /// one pass over the records.
    fn scan(&mut self) {
        if self.every.is_some() {
            return;
        }
        let mut every = self.empty();
        let mut scanned: Vec<Option<super::Hits>> = self
            .read
            .iter()
            .map(|column| match column {
                Column::Mark(_) | Column::Date(..) => Some(self.empty()),
                _ => None,
            })
            .collect();
        let any = scanned.iter().any(Option::is_some);
        let columns = self.columns;
        columns.each(|record| {
            if !self.in_scope(&record) {
                return;
            }
            let (word, bit) = (record.seq / 64, 1u64 << (record.seq % 64));
            every[record.account][word] |= bit;
            if !any {
                return;
            }
            for (column, hits) in self.read.iter().zip(scanned.iter_mut()) {
                let holds = match column {
                    Column::Mark(mark) => record.marks & mark == *mark,
                    Column::Date(since, until) => {
                        columns.is_mail(record.kind)
                            && record.date != 0
                            && (*since..*until).contains(&record.date)
                    }
                    _ => continue,
                };
                if holds && let Some(hits) = hits {
                    hits[record.account][word] |= bit;
                }
            }
        });
        self.every = Some(every);
        self.scanned = scanned;
    }

    fn every(&mut self) -> super::Hits {
        self.scan();
        self.every.clone().unwrap_or_default()
    }

    /// The hits of the bodies and summaries FTS5 expressions match, the
    /// summaries' only when `summary` is; those of the bodies
    /// [`proven`](Self::proven) under no negation.
    fn text(
        &mut self,
        body: Option<&str>,
        summary: Option<&str>,
    ) -> Result<super::Hits, PimdirError> {
        let mut hits = self.empty();
        let columns = self.columns;
        if let Some(expression) = body {
            let (sql, expression) = match self.index.posted_word(expression) {
                Some(word) => (search::MATCH_POSTING, word),
                None => (
                    "SELECT rowid FROM object_text WHERE object_text MATCH ?1 LIMIT ?2",
                    expression,
                ),
            };
            let mut statement = self.index.conn.prepare_cached(sql)?;
            let mut rows = statement.query(rusqlite::params![expression, -1])?;
            while let Some(row) = rows.next()? {
                columns.of_object(row.get(0)?, |record| {
                    if self.in_scope(&record) {
                        hits[record.account][record.seq / 64] |= 1 << (record.seq % 64);
                    }
                });
            }
            if self.negations == 0 {
                match self.proven.is_empty() {
                    true => self.proven = hits.clone(),
                    false => super::unite(&mut self.proven, &hits),
                }
            }
        }
        if let Some(expression) = summary {
            let mut statement = self
                .index
                .conn
                .prepare_cached("SELECT rowid FROM summary_text WHERE summary_text MATCH ?1")?;
            let mut rows = statement.query([expression])?;
            while let Some(row) = rows.next()? {
                if let Some(record) = columns.record(row.get(0)?)
                    && self.in_scope(&record)
                {
                    hits[record.account][record.seq / 64] |= 1 << (record.seq % 64);
                }
            }
        }
        Ok(hits)
    }

    /// The hits of the store items `address` stands in, in `role` or in
    /// any.
    fn address(&self, address: &str, role: Option<&str>) -> Result<super::Hits, PimdirError> {
        let mut hits = self.empty();
        let mut statement = self.index.conn.prepare_cached(
            "SELECT item FROM store.summary_value \
             WHERE value = ?1 AND (?2 IS NULL OR role = ?2) \
             AND role NOT IN ('in-reply-to', 'category')",
        )?;
        let mut rows = statement.query(rusqlite::params![address, role])?;
        while let Some(row) = rows.next()? {
            if let Some(record) = self.columns.of_item(row.get(0)?)
                && self.in_scope(&record)
            {
                hits[record.account][record.seq / 64] |= 1 << (record.seq % 64);
            }
        }
        Ok(hits)
    }

    fn leaf(&mut self, at: usize) -> Result<super::Hits, PimdirError> {
        let read = self.read;
        match &read[at] {
            Column::Every => Ok(self.every()),
            Column::Mark(_) | Column::Date(..) => {
                self.scan();
                Ok(self.scanned[at].clone().unwrap_or_default())
            }
            Column::Text(expression, summary) => {
                let summary = summary.then_some(*expression);
                self.text(Some(expression), summary)
            }
            Column::Address(address, role) => self.address(address, *role),
        }
    }

    /// Whether `node` is a flag or date term, or one negated, read by a
    /// scan of the records.
    fn scans(&self, node: &PimdirPlanNode) -> bool {
        let at = match node {
            PimdirPlanNode::Leaf(at) => *at,
            PimdirPlanNode::Not(operand) => match **operand {
                PimdirPlanNode::Leaf(at) => at,
                _ => return false,
            },
            _ => return false,
        };
        matches!(self.read[at], Column::Mark(_) | Column::Date(..))
    }

    /// Keeps the hits of `hits` meeting `node`, a flag or date term or one
    /// negated, read off each hit's held word rather than a scan, when
    /// they are few against the records and, for a date, every hit's
    /// dated mail placements share one date; whether it did.
    fn filters(&self, hits: &mut super::Hits, node: &PimdirPlanNode) -> bool {
        if self.every.is_some()
            || super::hits_counted(hits) as usize * 8 > self.columns.placements()
        {
            return false;
        }
        let (at, negated) = match node {
            PimdirPlanNode::Leaf(at) => (*at, false),
            PimdirPlanNode::Not(operand) => match **operand {
                PimdirPlanNode::Leaf(at) => (at, true),
                _ => return false,
            },
            _ => return false,
        };
        let column = &self.read[at];
        if matches!(column, Column::Date(..)) && !self.columns.uniform_dates {
            return false;
        }
        for (account, bits) in hits.iter_mut().enumerate() {
            for (word_at, word) in bits.iter_mut().enumerate() {
                let mut each = *word;
                while each != 0 {
                    let bit = each.trailing_zeros() as usize;
                    each &= each - 1;
                    let (marks, date) = self.columns.held(account, word_at * 64 + bit);
                    let holds = match column {
                        Column::Mark(mark) => marks & mark == *mark,
                        Column::Date(since, until) => date != 0 && (*since..*until).contains(&date),
                        _ => return false,
                    };
                    if holds == negated {
                        *word &= !(1 << bit);
                    }
                }
            }
        }
        true
    }

    /// Whether `node` is a text term filtered to one column (`{from} :`),
    /// which a joint `AND` steps through more slowly than a bare word,
    /// decoding each body's positions to find the column: FTS5 steps its
    /// first operand and seeks the others, so such a term goes last.
    fn column_filtered(&self, node: &PimdirPlanNode) -> bool {
        matches!(node, PimdirPlanNode::Leaf(at)
            if matches!(self.read[*at], Column::Text(expression, _) if expression.starts_with('{')))
    }

    fn is_text(&self, node: &PimdirPlanNode) -> bool {
        match node {
            PimdirPlanNode::Leaf(at) => matches!(self.read[*at], Column::Text(..)),
            PimdirPlanNode::Not(operand) => {
                matches!(**operand, PimdirPlanNode::Leaf(at) if matches!(self.read[at], Column::Text(..)))
            }
            _ => false,
        }
    }

    /// The hits of `node`, standing under one more negation.
    fn negated(&mut self, node: &PimdirPlanNode) -> Result<super::Hits, PimdirError> {
        self.negations += 1;
        let hits = self.eval(node);
        self.negations -= 1;
        hits
    }

    /// The hits of `node`, as [`flagged`](super::flagged) combines them:
    /// an `and` intersects its positive operands, its negated ones taken
    /// away, every hit of the scope standing in for none; its text
    /// operands matched as one expression when they can be.
    fn eval(&mut self, node: &PimdirPlanNode) -> Result<super::Hits, PimdirError> {
        match node {
            PimdirPlanNode::Leaf(at) => self.leaf(*at),
            PimdirPlanNode::Not(operand) => {
                let mut hits = self.every();
                super::subtract(&mut hits, &self.negated(operand)?);
                Ok(hits)
            }
            PimdirPlanNode::Or(operands) => {
                let mut hits = self.empty();
                for operand in operands {
                    super::unite(&mut hits, &self.eval(operand)?);
                }
                Ok(hits)
            }
            PimdirPlanNode::And(operands) => {
                let mut rest: Vec<&PimdirPlanNode> = operands.iter().collect();
                let mut hits: Option<super::Hits> = None;
                let text: Vec<PimdirPlanNode> = operands
                    .iter()
                    .filter(|operand| self.is_text(operand))
                    .cloned()
                    .collect();
                let positive = text
                    .iter()
                    .any(|operand| matches!(operand, PimdirPlanNode::Leaf(_)));
                if self.joint && text.len() >= 2 && positive {
                    let mut text = text;
                    text.sort_by_key(|operand| self.column_filtered(operand));
                    let node = PimdirPlanNode::And(text);
                    let body = self.compiled.jointly(&node, false);
                    let summary = self.compiled.jointly(&node, true);
                    hits = Some(self.text(body.as_deref(), summary.as_deref())?);
                    rest.retain(|operand| !self.is_text(operand));
                }
                rest.sort_by_key(|operand| self.scans(operand));
                for operand in rest
                    .iter()
                    .filter(|operand| !matches!(operand, PimdirPlanNode::Not(_)))
                {
                    if let Some(held) = &mut hits
                        && self.scans(operand)
                        && self.filters(held, operand)
                    {
                        continue;
                    }
                    let other = self.eval(operand)?;
                    match &mut hits {
                        None => hits = Some(other),
                        Some(hits) => super::intersect(hits, &other),
                    }
                }
                let mut hits = match hits {
                    Some(hits) => hits,
                    None => self.every(),
                };
                for operand in rest {
                    let PimdirPlanNode::Not(negated) = operand else {
                        continue;
                    };
                    if super::hits_counted(&hits) == 0 {
                        break;
                    }
                    if self.scans(operand) && self.filters(&mut hits, operand) {
                        continue;
                    }
                    super::subtract(&mut hits, &self.negated(negated)?);
                }
                Ok(hits)
            }
        }
    }
}

impl super::PimdirIndex {
    /// The expression of the posting lists answering `expression`, a
    /// `subject:` or `from:` term of one word or prefix as the compiler
    /// renders it (`{title} : "word"` or `{from} : "word"`, then ` *` for
    /// a prefix), when index.db holds the lists: the same word under the
    /// same tokenizer over the same text in the same column, so the bodies
    /// it matches are the column filter's. `None` for any other
    /// expression, a phrase or a word the tokenizer could split among
    /// them, which `object_text` answers.
    fn posted_word<'e>(&self, expression: &'e str) -> Option<&'e str> {
        if !self.posting {
            return None;
        }
        let word = expression
            .strip_prefix("{title} : ")
            .or_else(|| expression.strip_prefix("{from} : "))?;
        let quoted = word.strip_suffix(" *").unwrap_or(word);
        let inner = quoted.strip_prefix('"')?.strip_suffix('"')?;
        match !inner.is_empty() && inner.bytes().all(|b| b.is_ascii_alphanumeric()) {
            true => Some(expression),
            false => None,
        }
    }

    /// Reads each probed term of a page answered off the hit columns into
    /// bitmaps of the bodies and summaries it matches, one scan of the
    /// full-text index each, where a probe per hit would expand a prefix
    /// again or seek the term's matches once more: every prefix term, and
    /// a plain one matching at most
    /// [`PROBE_SET_MAX`](super::PROBE_SET_MAX) of each.
    pub(super) fn probe_sets(
        &self,
        probes: &mut [(&super::Probe, super::Sourced)],
    ) -> Result<(), PimdirError> {
        for (probe, sourced) in probes.iter_mut() {
            if !matches!(sourced, super::Sourced::Probed) {
                continue;
            }
            let limit = match probe.expression.contains('*')
                || self.posted_word(&probe.expression).is_some()
            {
                true => -1,
                false => super::PROBE_SET_MAX as i64 + 1,
            };
            let mut bodies = Vec::new();
            let mut summaries = Vec::new();
            let set = |bits: &mut Vec<u64>, id: i64| {
                let Ok(id) = usize::try_from(id) else {
                    return;
                };
                if bits.len() <= id / 64 {
                    bits.resize(id / 64 + 1, 0);
                }
                bits[id / 64] |= 1 << (id % 64);
            };
            let read =
                |sql: &str, expression: &str, bits: &mut Vec<u64>| -> Result<bool, PimdirError> {
                    let mut statement = self.conn.prepare_cached(sql)?;
                    let mut rows = statement.query(rusqlite::params![expression, limit])?;
                    let mut count = 0i64;
                    while let Some(row) = rows.next()? {
                        count += 1;
                        if count == limit {
                            return Ok(false);
                        }
                        set(bits, row.get(0)?);
                    }
                    Ok(true)
                };
            let (body, expression) = match self.posted_word(&probe.expression) {
                Some(word) => (search::MATCH_POSTING, word),
                None => (
                    "SELECT rowid FROM object_text WHERE object_text MATCH ?1 LIMIT ?2",
                    probe.expression.as_str(),
                ),
            };
            if !read(body, expression, &mut bodies)? {
                continue;
            }
            let summary = "SELECT rowid FROM summary_text WHERE summary_text MATCH ?1 LIMIT ?2";
            if probe.summary && !read(summary, &probe.expression, &mut summaries)? {
                continue;
            }
            *sourced = super::Sourced::Sets { bodies, summaries };
        }
        Ok(())
    }
}

/// Appends `text` to `out` as a JSON string, quoted and escaped.
fn json_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&alloc::format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}
