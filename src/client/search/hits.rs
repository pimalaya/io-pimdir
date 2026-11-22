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

use alloc::{string::String, vec, vec::Vec};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{Read, Write},
    path::Path,
};

use rusqlite::Connection;

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
pub(crate) const NAMES_MAX: usize = 64;

/// The most words the per-hit sort keys may take.
const HIT_KEYS_MAX: u64 = 1 << 28;

/// A hit's sort key when one of its placements is of an undated kind.
pub(crate) const UNDATED: i64 = -1;

/// No hit at that public id.
pub(crate) const NO_HIT: i64 = i64::MIN;

/// The media types holding no date (SEARCH §8.2).
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
        let (store_change, store_purges) = conn.query_row_cached(
            "SELECT store_change, store_purges FROM index_meta WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let placement_max =
            conn.query_row_cached("SELECT ifnull(max(id), 0) FROM placement", [], |row| {
                row.get(0)
            })?;
        let object_max =
            conn.query_row_cached("SELECT ifnull(max(id), 0) FROM object", [], |row| {
                row.get(0)
            })?;
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
    let ino = 0;
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

/// Writes the cache of `dir` stamped `stamp`, unless it already is, from
/// the index `conn` holds with the store attached, beside it then renamed
/// in; removes it when the store is past what a cache holds.
pub(crate) fn write(conn: &Connection, dir: &Path, stamp: Stamp) -> Result<(), PimdirError> {
    let path = dir.join(FILE);
    if let Ok(mut file) = File::open(&path) {
        let mut head = [0u8; 104];
        if file.read_exact(&mut head).is_ok() {
            let word = |at: usize| {
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(&head[at * 8..at * 8 + 8]);
                u64::from_ne_bytes(bytes)
            };
            if word(0) == MAGIC
                && [word(1), word(2), word(3), word(4)] == stamp.words()
                && word(12) == stamp.file
            {
                return Ok(());
            }
        }
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

    let mut out: Vec<u64> = vec![0; HEADER];
    out[0] = MAGIC;
    out[1..5].copy_from_slice(&stamp.words());
    out[12] = stamp.file;
    out[13] = items_len as u64;
    out[5] = records_len as u64;
    out[6] = objects_len as u64;
    out[7] = multi.len() as u64;
    out[8] = seq_max as u64;
    out[9] = accounts.len() as u64;
    out[10] = kinds.len() as u64;
    out[11] = if exact { EXACT_DATES } else { 0 } | if uniform { UNIFORM_DATES } else { 0 };
    put_names(&mut out, &accounts);
    put_names(&mut out, &kinds);
    out.extend_from_slice(&records);
    out.extend(
        objects
            .chunks(2)
            .map(|pair| u64::from(pair[0]) | u64::from(*pair.get(1).unwrap_or(&0)) << 32),
    );
    out.extend_from_slice(&multi);
    out.extend(
        items
            .chunks(2)
            .map(|pair| u64::from(pair[0]) | u64::from(*pair.get(1).unwrap_or(&0)) << 32),
    );
    out.extend(keys.iter().map(|key| *key as u64));
    out.extend_from_slice(&held);

    let new = dir.join(".index.hits.new");
    {
        let mut file = File::create(&new)?;
        // SAFETY: the words are plain integers viewed as their bytes.
        let bytes =
            unsafe { core::slice::from_raw_parts(out.as_ptr().cast::<u8>(), out.len() * 8) };
        file.write_all(bytes)?;
    }
    fs::rename(&new, &path)?;
    Ok(())
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
    /// account's name by its number: `None` when the index does not hold
    /// the store as it stands, no cache holds its state, or a term or the
    /// scope is one the columns cannot answer (a collection scope, a term
    /// other than system flags, text and dates over mail alone).
    pub(super) fn columnar(
        &self,
        compiled: &super::Compiled,
    ) -> Result<Option<(super::Hits, Vec<String>)>, PimdirError> {
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
        };
        let hits = run.eval(&plan.root)?;
        Ok(Some((hits, columns.accounts.clone())))
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
        let Some((hits, names)) = self.columnar(compiled)? else {
            return Ok(None);
        };
        self.columnar_batch(compiled, &hits, &names, after, sort, need)
    }

    /// The first `need` of `hits` after `after` in a dated `sort`: the
    /// hits whose sort key reaches the `need`-th past the cursor, those
    /// on the cursor's key counted in, written to `temp.hit_page`, then
    /// joined to the store, sorted and cut by one statement; the undated
    /// join them when fewer dated hits remain. `None` after an undated
    /// cursor, or past [`PAGE_HITS_MAX`](super::PAGE_HITS_MAX) of them.
    pub(super) fn columnar_batch(
        &self,
        compiled: &super::Compiled,
        hits: &super::Hits,
        names: &[String],
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

        self.conn
            .execute_batch(crate::sql::search::CREATE_HIT_PAGE)?;
        self.conn
            .execute_cached(crate::sql::search::CLEAR_HIT_PAGE, [])?;
        let mut insert = self
            .conn
            .prepare_cached(crate::sql::search::INSERT_HIT_PAGE)?;
        let mut failed = None;
        each(&mut |account, seq, key| {
            let wanted = match (key, threshold) {
                (UNDATED, _) => with_undated,
                (key, _) if !eligible(key) => false,
                (_, None) => true,
                (key, Some(threshold)) if newest => key >= threshold,
                (key, Some(threshold)) => key <= threshold,
            };
            if wanted && failed.is_none() {
                let name = names.get(account).map(String::as_str).unwrap_or("");
                if let Err(err) = insert.execute(rusqlite::named_params! {
                    ":akey": name,
                    ":seq": seq as i64,
                }) {
                    failed = Some(err);
                }
            }
        });
        drop(insert);
        if let Some(err) = failed {
            return Err(err.into());
        }

        let sql = compiled.met_rows(
            "SELECT akey, seq FROM temp.hit_page",
            sort,
            Some(after.is_some()),
        );
        let extra = super::page_params(after, need);
        Ok(Some(self.run(&sql, &compiled.values, &extra, |row| {
            super::found(row, sort)
        })?))
    }

    /// The members of `thread` in `account` among `hits`, in `sort`
    /// order, as [`members`](super::PimdirIndex::members) reads them.
    pub(super) fn columnar_members(
        &self,
        compiled: &super::Compiled,
        hits: &super::Hits,
        names: &[String],
        thread: &str,
        account: Option<&str>,
        sort: super::PimdirSearchSort,
    ) -> Result<Vec<super::Found>, PimdirError> {
        let name = account.unwrap_or_default();
        let Some(number) = names.iter().position(|known| known == name) else {
            return Ok(Vec::new());
        };
        let bits = &hits[number];
        let seqs: alloc::collections::BTreeSet<i64> = crate::client::rows(
            &self.conn,
            crate::sql::search::THREAD_MEMBERS,
            rusqlite::named_params! { ":thread": thread, ":account": account },
            |row| row.get::<_, i64>(1),
        )?
        .into_iter()
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
        let mut extra = vec![(
            String::from(":akey"),
            rusqlite::types::Value::from(String::from(name)),
        )];
        let mut listed = Vec::new();
        for (at, seq) in seqs.into_iter().enumerate() {
            listed.push(alloc::format!("(:m{at})"));
            extra.push((
                alloc::format!(":m{at}"),
                rusqlite::types::Value::Integer(seq),
            ));
        }
        let members = alloc::format!(
            "SELECT :akey AS akey, column1 AS seq FROM (VALUES {})",
            listed.join(", ")
        );
        let sql = compiled.met_rows(&members, sort, None);
        let mut found = self.run(&sql, &compiled.values, &extra, |row| {
            super::found(row, sort)
        })?;
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
    /// summaries' only when `summary` is.
    fn text(&self, body: Option<&str>, summary: Option<&str>) -> Result<super::Hits, PimdirError> {
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
            "SELECT item FROM store.item_address \
             WHERE address = ?1 AND (?2 IS NULL OR role = ?2)",
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
        match &self.read[at] {
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

    fn is_text(&self, node: &PimdirPlanNode) -> bool {
        match node {
            PimdirPlanNode::Leaf(at) => matches!(self.read[*at], Column::Text(..)),
            PimdirPlanNode::Not(operand) => {
                matches!(**operand, PimdirPlanNode::Leaf(at) if matches!(self.read[at], Column::Text(..)))
            }
            _ => false,
        }
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
                super::subtract(&mut hits, &self.eval(operand)?);
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
                    super::subtract(&mut hits, &self.eval(negated)?);
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
