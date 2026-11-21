//! # Search index
//!
//! index.db beside the store (SEARCH §2, §3): the full-text index over a
//! store's bodies and summaries, refreshed from its change feed and asked
//! in the query language of [`query`](crate::search::query).
//!
//! [`PimdirIndex`] is the indexer and the query client in one handle. It
//! opens index.db as its main database with pimdir.db attached read-only
//! as `store`, so one statement joins both, and rebuilds it rather than
//! migrating it when its version, tokenizer or shape is not this build's. A
//! refresh takes index.lock exclusively, on the owner lock's terms: one
//! held elsewhere is reported, never waited out. The index is derived:
//! dropping index.db loses nothing.
//!
//! The refresh is [`PimdirIndexRefresh`], the I/O-free coroutine an owner
//! holding its own database runs too, answered here through the canonical
//! statements one transaction per run.
//!
//! A refresh also keeps calendar time: every occurrence of a calendar
//! item within the horizon around its `now`, re-expanded as the horizon
//! rolls or the time zone database moves (SEARCH §7).
//!
//! A one-shot answer refreshes first within a deadline and says in its
//! coverage what it could not fold in; a session following the feed calls
//! [`refresh`](PimdirIndex::refresh) whenever the store moves.

mod compile;
#[cfg(feature = "indexer")]
mod refresh;
mod summary;
mod tag;

#[cfg(feature = "indexer")]
use core::mem;
use core::{
    cell::{Cell, RefCell},
    cmp::Ordering,
};

use alloc::{
    collections::{BTreeMap, BTreeSet},
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use std::{
    collections::HashMap,
    path::Path,
    sync::LazyLock,
    time::{SystemTime, UNIX_EPOCH},
};
#[cfg(feature = "indexer")]
use std::{
    fs::{self, File},
    io,
    path::PathBuf,
    time::Instant,
};

#[cfg(feature = "indexer")]
use fs4::FileExt;
use jiff::tz::TimeZone;
use log::{debug, trace};
#[cfg(feature = "indexer")]
use rusqlite::TransactionBehavior;
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Row, named_params,
    types::{Value, ValueRef},
};

pub use crate::search::plan::{PimdirSearchCursor, PimdirSearchSort};
use crate::{
    client::{
        PimdirError, PreparedCached, bind_list, busy_or_sql, modes, prepare_connection,
        reader::{PimdirReader, rolled_temporal},
        rows, schema,
        search::compile::{Categories, Compiled, Ends, Labels, Probe},
    },
    codec,
    coroutine::PimdirCoroutineState,
    object::PimdirHash,
    placement::{PimdirFlags, PimdirLevel, PimdirLinkId},
    property,
    search::{
        coverage::{self, PimdirSearchCoverage, PimdirSearchUnread},
        extract::{self, PimdirChunk, PimdirExtract, PimdirExtractStatus},
        fold,
        plan::{
            CAP, PimdirPlan, PimdirPlanAccess, PimdirPlanBound, PimdirPlanCosts, PimdirPlanDriver,
            PimdirPlanExtent, PimdirPlanForce, PimdirPlanNode, PimdirWalk, PimdirWalkCollection,
            PimdirWalkOutput, PimdirWalkRow,
        },
        query::{PimdirQuery, PimdirQueryKind, PimdirQueryPredicate},
    },
    sql::{self, search},
};
#[cfg(feature = "indexer")]
use crate::{
    client::{checkpoint, lock, search::refresh::Statements},
    search::refresh::PimdirIndexRefresh,
};

/// The file a refresh locks exclusively (SEARCH §3).
#[cfg(feature = "indexer")]
const LOCK: &str = "index.lock";

/// The highest public id a count marks in a bitmap per account (8 MiB
/// each); a store past it counts each placement's representative by a seek.
const BITMAP_SEQ_MAX: i64 = 1 << 26;

/// The most hits a page of text terms alone reads from its bitmaps, a
/// seek each for its sort key, rather than walking the store's order.
const PAGE_HITS_MAX: u64 = 40_000;

/// The values a dated page's statement binds beyond the query's: `:limit`
/// the `need` rows it reads, and the cursor `after` names.
fn page_params(after: Option<&PimdirSearchCursor>, need: usize) -> Vec<(String, Value)> {
    let mut extra = vec![(
        String::from(":limit"),
        Value::Integer(need.min(i64::MAX as usize) as i64),
    )];
    if let Some(after) = after {
        extra.extend([
            (
                String::from(":after_undated"),
                Value::Integer(i64::from(after.undated)),
            ),
            (
                String::from(":after_key"),
                Value::from(after.sort_key.clone()),
            ),
            (String::from(":after_seq"), Value::Integer(after.seq)),
            (
                String::from(":after_account"),
                Value::from(after.account.clone()),
            ),
        ]);
    }
    extra
}

/// Hits as bitmaps of public ids, one per account by its number.
type Hits = Vec<Vec<u64>>;

/// Marks the hit `seq` of account `account`.
fn hit(hits: &mut Hits, account: usize, seq: i64) {
    let Some(bits) = hits.get_mut(account) else {
        return;
    };
    let seq = seq.max(0) as usize;
    let word = seq / 64;
    if bits.len() <= word {
        bits.resize(word + 1, 0);
    }
    bits[word] |= 1u64 << (seq % 64);
}

/// The most split public ids a text count evaluates per placement (`split`,
/// SEARCH §3); past it each term's placements are read whole.
const SPLIT_MAX: i64 = 1024;

/// Unmarks the hit `seq` of every account.
fn forget(hits: &mut Hits, seq: i64) {
    let seq = seq.max(0) as usize;
    for bits in hits {
        if let Some(word) = bits.get_mut(seq / 64) {
            *word &= !(1u64 << (seq % 64));
        }
    }
}

/// Whether `node` holds for a hit whose leaves hold as `terms` says.
fn holds(node: &PimdirPlanNode, terms: &[bool]) -> bool {
    match node {
        PimdirPlanNode::Leaf(at) => terms[*at],
        PimdirPlanNode::Not(operand) => !holds(operand, terms),
        PimdirPlanNode::And(operands) => operands.iter().all(|operand| holds(operand, terms)),
        PimdirPlanNode::Or(operands) => operands.iter().any(|operand| holds(operand, terms)),
    }
}

/// Adds the hits of `other` to `hits`.
fn unite(hits: &mut Hits, other: &Hits) {
    for (bits, other) in hits.iter_mut().zip(other) {
        if bits.len() < other.len() {
            bits.resize(other.len(), 0);
        }
        for (word, other) in bits.iter_mut().zip(other) {
            *word |= other;
        }
    }
}

/// Keeps the hits of `hits` that `other` holds.
fn intersect(hits: &mut Hits, other: &Hits) {
    for (bits, other) in hits.iter_mut().zip(other) {
        bits.truncate(other.len());
        for (word, other) in bits.iter_mut().zip(other) {
            *word &= other;
        }
    }
}

/// Takes the hits of `other` away from `hits`.
fn subtract(hits: &mut Hits, other: &Hits) {
    for (bits, other) in hits.iter_mut().zip(other) {
        for (word, other) in bits.iter_mut().zip(other) {
            *word &= !other;
        }
    }
}

/// How many hits `hits` holds.
fn hits_counted(hits: &Hits) -> u64 {
    hits.iter()
        .flatten()
        .map(|word| u64::from(word.count_ones()))
        .sum()
}

/// The hits of `node` among `every`, each leaf's from `leaf`: `not`
/// the hits of `every` its operand leaves out.
fn flagged(node: &PimdirPlanNode, every: &Hits, leaf: &impl Fn(usize) -> Hits) -> Hits {
    match node {
        PimdirPlanNode::Leaf(at) => leaf(*at),
        PimdirPlanNode::Not(operand) => {
            let mut hits = every.clone();
            subtract(&mut hits, &flagged(operand, every, leaf));
            hits
        }
        PimdirPlanNode::And(operands) => {
            let mut hits = every.clone();
            for operand in operands {
                intersect(&mut hits, &flagged(operand, every, leaf));
            }
            hits
        }
        PimdirPlanNode::Or(operands) => {
            let mut hits = vec![Vec::new(); every.len()];
            for operand in operands {
                unite(&mut hits, &flagged(operand, every, leaf));
            }
            hits
        }
    }
}

/// The collections of the store by id, each with its account's number,
/// for a count marking hits per account; none read when the store holds
/// one account, every row then being its.
struct Held {
    /// The number of each collection's account.
    collections: HashMap<String, usize>,
    /// How many accounts there are.
    accounts: usize,
}

impl Held {
    /// The number of the account a row's collection, its first column,
    /// belongs to; `None` for a collection the store does not hold.
    fn account(&self, row: &Row) -> rusqlite::Result<Option<usize>> {
        if self.accounts == 1 {
            return Ok(Some(0));
        }
        let ValueRef::Text(collection) = row.get_ref(0)? else {
            return Ok(None);
        };
        Ok(core::str::from_utf8(collection)
            .ok()
            .and_then(|collection| self.of(collection)))
    }

    /// The number of the account collection `collection` belongs to;
    /// `None` for a collection the store does not hold.
    fn of(&self, collection: &str) -> Option<usize> {
        if self.accounts == 1 {
            return Some(0);
        }
        self.collections.get(collection).copied()
    }
}

/// The tables, indexes and triggers the index schema declares, by name,
/// as [`shape`] reads them from an index.db (SEARCH §2).
static SHAPE_DECLARED: LazyLock<BTreeMap<String, String>> =
    LazyLock::new(|| schema::declared(search::MIGRATIONS));

/// The SQLite version the index requires, for `contentless_delete`
/// (SEARCH §2), as `sqlite3_libversion_number` spells it.
const SQLITE_MIN: i32 = 3_043_000;

/// About how many characters a snippet holds.
const SNIPPET_LEN: usize = 200;

/// A store's search index: the indexer and the query client.
pub struct PimdirIndex {
    conn: Connection,
    reader: PimdirReader,
    #[cfg(feature = "indexer")]
    dir: PathBuf,
    /// Opened by [`open_query`](PimdirIndex::open_query): index.db read
    /// only, never refreshed.
    #[cfg(feature = "indexer")]
    query_only: bool,
    /// Whether the index stands under this build's extraction (SEARCH §2,
    /// §6), so the chunk offsets it stores index the text this build
    /// extracts; a query client re-chunks its own text otherwise.
    own_extraction: bool,
    /// The coverages answered since index.db and the store last moved.
    coverages: RefCell<PimdirCoverageCache>,
    /// The driver every answer takes in place of the plan's, set by
    /// [`force_driver`](PimdirIndex::force_driver).
    force: Cell<Option<PimdirPlanForce>>,
}

/// The most coverages a handle keeps between two moves of index.db or the
/// store, the cache emptied when one more is read.
const COVERAGES_MAX: usize = 64;

/// What a query's coverage is read over (SEARCH §8.2): the conjuncts
/// narrowing its collections, the date range they narrow it to and
/// whether a date term stands anywhere in it. Queries of one scope read
/// one coverage from the store and the index, so it keys the cache.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct PimdirCoverageScope {
    /// The conjuncts ruling collections out, in reading order.
    terms: Vec<PimdirCoverageTerm>,
    /// The range's start, RFC 3339, `None` open.
    since: Option<String>,
    /// The range's end, exclusive, `None` open.
    until: Option<String>,
    /// Whether a date term stands in the query, wherever it stands.
    dated: bool,
}

/// A conjunct ruling collections out of a coverage (SEARCH §8.2).
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum PimdirCoverageTerm {
    /// `account:`, the account's name.
    Account(String),
    /// `collection:` or `folder:`, the collection's id.
    Collection(String),
    /// `kind:`, the media type it names.
    Kind(&'static str),
}

impl PimdirCoverageScope {
    /// The scope of `query`, its dates resolved against `now` in `zone`.
    fn of(query: &PimdirQuery, now: i64, zone: &TimeZone) -> Self {
        let mut terms = Vec::new();
        let mut since: Option<String> = None;
        let mut until: Option<String> = None;
        for term in query.conjuncts() {
            match &term.predicate {
                PimdirQueryPredicate::Account(account) => {
                    terms.push(PimdirCoverageTerm::Account(account.clone()))
                }
                PimdirQueryPredicate::Collection(id) => {
                    terms.push(PimdirCoverageTerm::Collection(id.clone()))
                }
                PimdirQueryPredicate::Kind(kind) => {
                    terms.push(PimdirCoverageTerm::Kind(compile::media_type(*kind)))
                }
                PimdirQueryPredicate::Date(range) => {
                    let (start, end) = compile::bounds(range, now, zone);
                    if start > since {
                        since = start;
                    }
                    if let Some(end) = end
                        && until.as_ref().is_none_or(|until| &end < until)
                    {
                        until = Some(end);
                    }
                }
                _ => (),
            }
        }
        let dated = query
            .terms()
            .into_iter()
            .any(|(term, _)| matches!(term.predicate, PimdirQueryPredicate::Date(_)));
        Self {
            terms,
            since,
            until,
            dated,
        }
    }

    /// Whether the collection of `id`, `account` and `kind` stands in the
    /// scope.
    fn holds(&self, id: &str, account: Option<&str>, kind: &str) -> bool {
        self.terms.iter().all(|term| match term {
            PimdirCoverageTerm::Account(name) => account == Some(name.as_str()),
            PimdirCoverageTerm::Collection(name) => id == name,
            PimdirCoverageTerm::Kind(media_type) => kind == *media_type,
        })
    }
}

/// A coverage as the store and the index hold it, before the query's own
/// reads and zone.
#[derive(Clone, Debug)]
struct PimdirCoverageHeld {
    /// The coverage, its `reads` and `zone_differs` left to the query.
    coverage: PimdirSearchCoverage,
    /// `index_meta.floating_zone`, read when a date term of the query can
    /// reach a calendar collection of its scope, `None` otherwise.
    floating: Option<String>,
}

/// The coverages one handle read since index.db and the store last moved,
/// by scope: a second answer over a scope with no commit between reads no
/// row of either (SEARCH §8.2).
#[derive(Debug, Default)]
struct PimdirCoverageCache {
    /// `PRAGMA data_version` of index.db and of the store when the held
    /// coverages were read: another connection's commit moves either,
    /// this one's own writes being forgotten by the refresh making them.
    versions: Option<(i64, i64)>,
    /// The coverages read since, by scope.
    held: BTreeMap<PimdirCoverageScope, PimdirCoverageHeld>,
}

/// How far one refresh goes.
#[cfg(feature = "indexer")]
#[derive(Clone, Debug)]
pub struct PimdirIndexRefreshOptions {
    /// The most feed rows one transaction folds, so the store's WAL still
    /// checkpoints between them (SEARCH §4). 4,096 by default: each commit
    /// rewrites a page of every random-key index per row it touched, so a
    /// first build at 256 wrote 3.7 times the bytes, at 28 MB more memory.
    pub batch: usize,
    /// When to stop: checked between batches, so the pass ends at the
    /// first batch boundary past it with the cursor at that boundary.
    /// `None` folds the whole feed.
    pub deadline: Option<Instant>,
    /// The instant the horizon is drawn around (SEARCH §7), Unix seconds;
    /// `None` for the clock's.
    pub now: Option<i64>,
    /// The IANA zone floating times and dates are read in (SEARCH §7),
    /// `UTC` by default: one other than the index records re-expands
    /// every live calendar item.
    pub floating_zone: String,
}

#[cfg(feature = "indexer")]
impl Default for PimdirIndexRefreshOptions {
    fn default() -> Self {
        Self {
            batch: 4096,
            deadline: None,
            now: None,
            floating_zone: coverage::FLOATING_ZONE.into(),
        }
    }
}

/// What one refresh did.
#[cfg(feature = "indexer")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PimdirIndexReport {
    /// Feed rows folded.
    pub folded: u64,
    /// Bodies extracted and indexed for the first time.
    pub indexed: u64,
    /// Calendar items whose occurrences were expanded (SEARCH §7).
    pub expanded: u64,
    /// Calendar items cut at `OCCURRENCES_MAX` occurrences.
    pub bounded: u64,
    /// Placements dropped: deleted, retained, purged or renamed away.
    pub removed: u64,
    /// Whether the key reconciliation ran (SEARCH §4).
    pub reconciled: bool,
    /// The store's last stamp when the pass began.
    pub head: i64,
    /// The index cursor the pass left: every stamp up to it is folded.
    pub cursor: i64,
    /// Whether another indexer held index.lock, so this one did nothing.
    pub busy: bool,
}

#[cfg(feature = "indexer")]
impl PimdirIndexReport {
    /// Whether the index has folded every change the store had when the
    /// pass began.
    pub fn is_current(&self) -> bool {
        !self.busy && self.cursor >= self.head
    }
}

/// One page of a search.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirSearchOptions {
    /// The order.
    pub sort: PimdirSearchSort,
    /// The most hits answered.
    pub limit: usize,
    /// Where the page starts: after the hit, or the thread, an earlier
    /// page ended on (SEARCH §8.2), its `next`; `None` for the first.
    pub after: Option<PimdirSearchCursor>,
    /// The instant and the zone the query's dates resolve in.
    pub clock: PimdirSearchClock,
}

impl Default for PimdirSearchOptions {
    fn default() -> Self {
        Self {
            sort: PimdirSearchSort::NewestFirst,
            limit: 50,
            after: None,
            clock: PimdirSearchClock::default(),
        }
    }
}

/// The instant and the zone a query's dates resolve in (SEARCH §8.1).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirSearchClock {
    /// The query's now, Unix seconds; `None` for the clock's.
    pub now: Option<i64>,
    /// The IANA zone of its calendar spans, `None` for UTC.
    pub zone: Option<String>,
}

impl PimdirSearchClock {
    /// The query's now, Unix seconds.
    pub fn now(&self) -> i64 {
        self.now.unwrap_or_else(unix_now)
    }

    /// The query's zone, [`PimdirError::IndexZone`] when unknown.
    fn time_zone(&self) -> Result<TimeZone, PimdirError> {
        match self.zone.as_deref() {
            None => Ok(TimeZone::UTC),
            Some(zone) => TimeZone::get(zone).map_err(|_| PimdirError::IndexZone(zone.into())),
        }
    }
}

/// A threaded listing's answer (SEARCH §9): one page of the hits
/// collapsed to their threads, and what it could not see.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirSearchThreadsOutput {
    /// The threads, in the order of their first hit.
    pub threads: Vec<PimdirSearchThread>,
    /// Whether threads follow this page.
    pub more: bool,
    /// Where the next page starts, when one follows: the place of the
    /// first hit of this page's last thread.
    pub next: Option<PimdirSearchCursor>,
    /// What the answer could not see (SEARCH §8.2).
    pub coverage: PimdirSearchCoverage,
}

/// One occurrence of a calendar item (SEARCH §7), as an agenda reads a
/// range: the placement it belongs to and its instants, RFC 3339 in UTC.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirSearchOccurrence {
    /// The collection holding the placement.
    pub collection: String,
    /// The placement's `seq` in it.
    pub seq: i64,
    /// When the occurrence starts.
    pub start: String,
    /// When it ends, its start for one of zero length.
    pub end: String,
}

/// The hits of one thread in one account (SEARCH §9), or a hit no thread
/// holds (an item other than mail) alone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirSearchThread {
    /// The account, `None` in a single-account store.
    pub account: Option<String>,
    /// The thread id, as `thread:` takes it: the link id of its present
    /// member with the lowest sort key; `None` for a hit no thread holds.
    pub thread: Option<String>,
    /// The thread's live messages in the account, hit or not; 1 for a hit
    /// no thread holds.
    pub count: u64,
    /// The earliest sort key naming a date among them, `None` without
    /// one or outside a thread.
    pub first: Option<String>,
    /// The latest.
    pub last: Option<String>,
    /// The thread's hits: in the order asked in a listing, oldest first
    /// from [`PimdirIndex::thread`].
    pub hits: Vec<PimdirSearchHit>,
}

/// A search's answer: one page of hits and what it could not see.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirSearchOutput {
    /// The hits, in the order asked.
    pub hits: Vec<PimdirSearchHit>,
    /// Whether hits follow this page.
    pub more: bool,
    /// Where the next page starts, when one follows: the place of this
    /// page's last hit.
    pub next: Option<PimdirSearchCursor>,
    /// What the answer could not see (SEARCH §8.2).
    pub coverage: PimdirSearchCoverage,
}

/// A count's answer.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirSearchCountOutput {
    /// How many hits the query has.
    pub count: u64,
    /// What the count could not see (SEARCH §8.2).
    pub coverage: PimdirSearchCoverage,
}

/// One hit (SEARCH §8.2): an item in one account, addressed as
/// `(account, seq)`, with every live placement of it there.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirSearchHit {
    /// The account, `None` in a single-account store.
    pub account: Option<String>,
    /// The public id (STORAGE §9.1).
    pub seq: i64,
    /// The kind, as `kind:` names it, or the collection's media type for
    /// one the query language has no name for.
    pub kind: String,
    /// Its subject, name or summary.
    pub title: String,
    /// The latest sort key of its placements.
    pub sort_key: String,
    /// Its live placements in the account, in collection order.
    pub placements: Vec<PimdirSearchPlacement>,
    /// What a text term matched: its body or its summary; `None` when only
    /// structured terms did.
    pub matched: Option<PimdirSearchMatched>,
    /// A passage of its body, re-read from the blob: from the first chunk
    /// naming a query word, else the first; `None` without an indexed
    /// body or a chunk, or when the blob cannot be read.
    pub snippet: Option<String>,
}

/// One live placement of a hit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirSearchPlacement {
    /// The collection.
    pub collection: String,
    /// The item's key.
    pub link_id: PimdirLinkId,
    /// Its flags there.
    pub flags: PimdirFlags,
    /// The body's hash, `None` while bodiless.
    pub object: Option<PimdirHash>,
    /// The detail tier reached.
    pub level: PimdirLevel,
    /// Its sort key.
    pub sort_key: String,
}

/// What a text term matched (SEARCH §8.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PimdirSearchMatched {
    /// The indexed body.
    Body,
    /// The summary of a placement holding no body.
    Summary,
}

impl PimdirSearchMatched {
    /// The spelling the JSON output uses.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::Summary => "summary",
        }
    }
}

/// An item as the index reads it: the hit's address and placements, and
/// its text by field as extraction gives it (SEARCH §6).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirSearchItem {
    /// The account, `None` in a single-account store.
    pub account: Option<String>,
    /// The public id.
    pub seq: i64,
    /// The kind, as [`PimdirSearchHit::kind`].
    pub kind: String,
    /// Its subject, name or summary.
    pub title: String,
    /// Its live placements in the account.
    pub placements: Vec<PimdirSearchPlacement>,
    /// What the text was read from: the body, or the summary of an item
    /// holding none.
    pub source: PimdirSearchMatched,
    /// The text by field.
    pub extract: PimdirExtract,
    /// The chunks of its `body` and `note` fields.
    pub chunks: Vec<PimdirChunk>,
}

/// A hit as a compiled query answers it, before it is described.
#[derive(Clone)]
struct Found {
    account: Option<String>,
    seq: i64,
    /// Its place in the order asked.
    place: PimdirSearchCursor,
}

impl PimdirIndex {
    /// Opens the index of the store rooted at `dir`, creating index.db
    /// when absent and rebuilding it when its version, tokenizer or shape is
    /// not this build's, or when it stands under another revision of the
    /// extraction than [`EXTRACTION`](extract::EXTRACTION) and
    /// [`EXTRACTOR`](extract::EXTRACTOR), so every body is extracted and
    /// every calendar item expanded again (SEARCH §2).
    ///
    /// The store must exist ([`PimdirError::Uncreated`]). A SQLite without
    /// FTS5 or older than 3.43 is [`PimdirError::Fts5`]; another indexer
    /// holding index.lock while this one has to build is
    /// [`PimdirError::Indexing`].
    #[cfg(feature = "indexer")]
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, PimdirError> {
        let dir = dir.as_ref();
        debug!("opening search index at {}", dir.display());

        let reader = PimdirReader::open(dir)?;
        if rusqlite::version_number() < SQLITE_MIN {
            return Err(PimdirError::Fts5 {
                version: rusqlite::version(),
            });
        }

        let mut conn = connect(dir, false)?;
        if held(&conn, dir)? != PimdirIndexHeld::Current {
            let Some(_lock) = lock(dir)? else {
                return Err(PimdirError::Indexing(dir.to_path_buf()));
            };
            // NOTE: another indexer may have built it between the check
            // and the lock.
            if held(&conn, dir)? != PimdirIndexHeld::Current {
                drop(conn);
                conn = build(dir)?;
            }
        }
        conn.execute_batch("PRAGMA main.journal_mode = WAL;")?;

        Ok(Self {
            conn,
            reader,
            #[cfg(feature = "indexer")]
            dir: dir.to_path_buf(),
            #[cfg(feature = "indexer")]
            query_only: false,
            own_extraction: true,
            coverages: RefCell::default(),
            force: Cell::new(None),
        })
    }

    /// Opens the index of the store rooted at `dir` for queries alone
    /// (SEARCH §3): index.db read-only, no lock, never created, rebuilt
    /// nor refreshed, so a frontend answers from the index the store's
    /// indexer (its owner, at sync time) keeps.
    ///
    /// The store must exist ([`PimdirError::Uncreated`]). An index.db
    /// absent, or holding no schema, is [`PimdirError::IndexMissing`]; one
    /// at another schema version, tokenizer or shape, which
    /// [`open`](Self::open) would rebuild, is
    /// [`PimdirError::IndexOutdated`]: either way, run the indexer. One
    /// standing under another revision of the extraction or another
    /// indexer's own (`index_meta.extraction`, `extractor`, SEARCH §2) is
    /// answered from, its snippets cut from this build's own chunks
    /// rather than the stored offsets (SEARCH §6).
    /// [`refresh`](Self::refresh) on such a handle is
    /// [`PimdirError::IndexQueryOnly`], and every answer says in its
    /// coverage how far the index is behind the store
    /// ([`behind`](crate::search::coverage::PimdirSearchLocal::behind)).
    pub fn open_query(dir: impl AsRef<Path>) -> Result<Self, PimdirError> {
        let dir = dir.as_ref();
        debug!("opening search index at {} for queries", dir.display());

        let reader = PimdirReader::open(dir)?;
        if rusqlite::version_number() < SQLITE_MIN {
            return Err(PimdirError::Fts5 {
                version: rusqlite::version(),
            });
        }

        let path = dir.join("index.db");
        if !path.is_file() {
            return Err(PimdirError::IndexMissing(path));
        }
        let conn = connect(dir, true)?;
        let held = held(&conn, dir)?;
        match held {
            PimdirIndexHeld::Current | PimdirIndexHeld::Extracted => (),
            PimdirIndexHeld::Empty => return Err(PimdirError::IndexMissing(path)),
            PimdirIndexHeld::Earlier { version, shape } => {
                return Err(PimdirError::IndexVersion {
                    path,
                    version,
                    shape,
                });
            }
            PimdirIndexHeld::Stale => return Err(PimdirError::IndexOutdated(path)),
        }

        Ok(Self {
            conn,
            reader,
            #[cfg(feature = "indexer")]
            dir: dir.to_path_buf(),
            #[cfg(feature = "indexer")]
            query_only: true,
            own_extraction: held == PimdirIndexHeld::Current,
            coverages: RefCell::default(),
            force: Cell::new(None),
        })
    }

    /// The store's reader this index reads summaries, coverage and bodies
    /// through.
    pub fn reader(&self) -> &PimdirReader {
        &self.reader
    }

    /// The store's last stamp this index has folded in (SEARCH §4).
    pub fn cursor(&self) -> Result<i64, PimdirError> {
        Ok(self
            .conn
            .query_row_cached(search::LOAD_INDEX_META, [], |row| row.get(2))?)
    }

    /// The IANA zone the index reads floating times in (SEARCH §7).
    pub fn floating_zone(&self) -> Result<String, PimdirError> {
        let zone: Option<String> =
            self.conn
                .query_row_cached(search::LOAD_INDEX_META, [], |row| row.get(11))?;
        Ok(zone.unwrap_or_else(|| coverage::FLOATING_ZONE.into()))
    }

    /// Folds the store's change feed into the index (SEARCH §4), in
    /// transactions of at most `options.batch` rows, until the feed is
    /// exhausted or the deadline passes.
    ///
    /// Each transaction is one run of [`PimdirIndexRefresh`], its yields
    /// answered and its ops written through the canonical statements, so
    /// this index and one an owner keeps on its own SQLite fold alike. Each
    /// writes the cursor it reached, so a pass cut short resumes there; the
    /// one exhausting the feed also reconciles keys when the store's purge
    /// count moved, a rename counting there, meets again the bodies an earlier
    /// pass could not read, and writes the cursor read before it, so a
    /// crash replays and skips nothing. Another indexer holding index.lock
    /// makes this one return at once, [`busy`](PimdirIndexReport::busy). A
    /// handle opened by [`open_query`](Self::open_query) refreshes nothing:
    /// [`PimdirError::IndexQueryOnly`].
    ///
    /// An owner keeping an index calls it after each write batch it
    /// commits (SEARCH §4), so its lists' filters trail their rows by one
    /// batch. The store does not call it itself: the index is optional, a
    /// feature of its own, and a store never depends on it.
    #[cfg(feature = "indexer")]
    pub fn refresh(
        &mut self,
        options: &PimdirIndexRefreshOptions,
    ) -> Result<PimdirIndexReport, PimdirError> {
        if self.query_only {
            return Err(PimdirError::IndexQueryOnly(self.dir.join("index.db")));
        }
        *self.coverages.get_mut() = PimdirCoverageCache::default();
        let zone = options.floating_zone.as_str();
        if zone != coverage::FLOATING_ZONE && TimeZone::get(zone).is_err() {
            return Err(PimdirError::IndexZone(zone.into()));
        }
        let mut report = PimdirIndexReport {
            cursor: self.cursor()?,
            ..Default::default()
        };
        report.head = self.reader.change_cursor()?.changed;

        let Some(_lock) = lock(&self.dir)? else {
            debug!("another indexer holds {LOCK}, skipping the refresh");
            report.busy = true;
            return Ok(report);
        };
        // NOTE: the store may have been restored, or another store put in
        // its place, since this handle opened the index (SEARCH §4).
        if held(&self.conn, &self.dir)? != PimdirIndexHeld::Current {
            let held = mem::replace(&mut self.conn, Connection::open_in_memory()?);
            drop(held);
            self.conn = build(&self.dir)?;
            report.cursor = 0;
        }
        debug!(
            "refreshing search index from stamp {} to {}",
            report.cursor, report.head
        );

        let batch = options.batch.max(1);
        let blobs = self.reader.blobs();
        let now = options.now.unwrap_or_else(unix_now);
        let mut first = true;
        loop {
            if options
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                debug!("refresh deadline passed at stamp {}", report.cursor);
                break;
            }

            let tx = self
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(busy_or_sql)?;
            let statements = Statements {
                tx: &tx,
                blobs: &blobs,
            };
            let mut run = PimdirIndexRefresh::new(now, batch);
            run.floating_zone.clone_from(&options.floating_zone);
            run.missing = mem::take(&mut first);
            let mut arg = None;
            let output = loop {
                match run.resume(arg.take()) {
                    PimdirCoroutineState::Yielded(want) => arg = Some(statements.answer(want)?),
                    PimdirCoroutineState::Complete(output) => {
                        break output.map_err(PimdirError::Refresh)?;
                    }
                }
            };
            tx.commit().map_err(busy_or_sql)?;

            report.folded += output.folded;
            report.indexed += output.indexed;
            report.expanded += output.expanded;
            report.bounded += output.bounded;
            report.removed += output.removed;
            report.reconciled |= output.reconciled;
            report.cursor = output.cursor;
            if output.exhausted {
                break;
            }
        }

        debug!(
            "refreshed search index: {} rows folded, {} bodies indexed, cursor {}",
            report.folded, report.indexed, report.cursor
        );
        Ok(report)
    }

    /// Runs the index's part of an erase (STORAGE §11.5, SEARCH §4), after
    /// `PimdirStore::erase` purged the item: a refresh folding the whole
    /// feed in the zone the index records, which deletes the item's
    /// placement, its bodies and the thread rows it gave, then
    /// `optimize_text` under index.lock, merging every full-text segment
    /// so no deleted token stays in one, then a `TRUNCATE` checkpoint of
    /// index.db until every frame is checkpointed.
    ///
    /// Another indexer holding index.lock is [`PimdirError::Indexing`]; a
    /// handle opened by [`open_query`](Self::open_query) is
    /// [`PimdirError::IndexQueryOnly`].
    #[cfg(feature = "indexer")]
    pub fn erase(&mut self) -> Result<PimdirIndexReport, PimdirError> {
        let options = PimdirIndexRefreshOptions {
            floating_zone: self.floating_zone()?,
            ..Default::default()
        };
        let report = self.refresh(&options)?;

        let Some(_lock) = lock(&self.dir)?.filter(|_| !report.busy) else {
            return Err(PimdirError::Indexing(self.dir.clone()));
        };
        debug!("optimize the full-text tables");
        self.conn
            .execute_batch(search::OPTIMIZE_TEXT)
            .map_err(busy_or_sql)?;
        checkpoint(&self.conn)?;

        Ok(report)
    }

    /// One page of the hits of a query (SEARCH §8.2), with what it could
    /// not see. Answers from the index as it stands: refresh first.
    ///
    /// The plan reads the query's smallest set when a page of it costs
    /// less than a walk, else walks the scope's collections in the
    /// store's order until the page is full ([`PimdirWalk`]); the
    /// `title`, `sender` and `relevance` orders read every hit.
    pub fn search(
        &self,
        query: &PimdirQuery,
        options: &PimdirSearchOptions,
    ) -> Result<PimdirSearchOutput, PimdirError> {
        let now = options.clock.now();
        let zone = options.clock.time_zone()?;
        let (mut compiled, driver) = self.planned(query, now, &zone, options)?;
        let (page, more) = self.ordered(
            &mut compiled,
            &driver,
            options.after.as_ref(),
            options.sort,
            options.limit,
        )?;
        let next = match more {
            true => page
                .last()
                .map(|hit| hit.place.clone())
                .or(options.after.clone()),
            false => None,
        };

        let probes = self.presented(&compiled, page.len())?;
        let mut hits = Vec::new();
        for hit in page {
            hits.push(self.hit(hit, &probes, &compiled.words)?);
        }
        let coverage = self.coverage(query, now, &options.clock, &zone)?;

        Ok(PimdirSearchOutput {
            hits,
            more,
            next,
            coverage,
        })
    }

    /// One page of the threaded listing of a query (SEARCH §9): its hits
    /// collapsed to their thread in their account, each thread with its
    /// count, first and last date, the threads in the order of their
    /// first hit and paged by `options.limit` after `options.after`, the
    /// first hit of the last thread an earlier page answered. A hit no
    /// thread holds, an item other than mail, stands alone.
    ///
    /// The hits are read in order a batch at a time, as a page is, until
    /// the page holds its threads and one more: each thread met is read
    /// whole, its hits among its members, and one whose first hit stands
    /// at or before the cursor is an earlier page's.
    pub fn threads(
        &self,
        query: &PimdirQuery,
        options: &PimdirSearchOptions,
    ) -> Result<PimdirSearchThreadsOutput, PimdirError> {
        let now = options.clock.now();
        let zone = options.clock.time_zone()?;
        let (mut compiled, driver) = self.planned(query, now, &zone, options)?;
        let sort = options.sort;
        let batch = match sort.is_dated() {
            true => options.limit.saturating_mul(2).max(1),
            false => usize::MAX - 1,
        };
        let wanted = options.limit.saturating_add(1);

        let mut groups: Vec<(Option<String>, Vec<Found>)> = Vec::new();
        let mut met: BTreeSet<(Option<String>, String)> = BTreeSet::new();
        let mut cursor = options.after.clone();
        'batches: loop {
            let (found, more) =
                self.ordered(&mut compiled, &driver, cursor.as_ref(), sort, batch)?;
            for hit in found {
                cursor = Some(hit.place.clone());
                let thread: Option<String> = self
                    .conn
                    .query_row_cached(
                        search::HIT_THREAD,
                        named_params! { ":seq": hit.seq, ":account": hit.account },
                        |row| row.get(0),
                    )
                    .optional()?;
                let Some(thread) = thread else {
                    groups.push((None, vec![hit]));
                    if groups.len() >= wanted {
                        break 'batches;
                    }
                    continue;
                };
                if !met.insert((hit.account.clone(), thread.clone())) {
                    continue;
                }
                let members = self.members(&compiled, &thread, hit.account.as_deref(), sort)?;
                let earlier = options.after.as_ref().is_some_and(|after| {
                    members
                        .first()
                        .is_some_and(|first| first.place.compare(after, sort) != Ordering::Greater)
                });
                if earlier || members.is_empty() {
                    continue;
                }
                groups.push((Some(thread), members));
                if groups.len() >= wanted {
                    break 'batches;
                }
            }
            if !more {
                break;
            }
        }
        let more = groups.len() > options.limit;
        groups.truncate(options.limit);
        let next = match more {
            true => groups
                .last()
                .map(|(_, hits)| hits[0].place.clone())
                .or(options.after.clone()),
            false => None,
        };

        let shown = groups.iter().map(|(_, found)| found.len()).sum();
        let probes = self.presented(&compiled, shown)?;
        let mut threads = Vec::new();
        for (thread, found) in groups {
            let account = found[0].account.clone();
            let (count, first, last) = match &thread {
                Some(thread) => self.conn.query_row_cached(
                    search::THREAD_SPAN,
                    named_params! { ":thread": thread, ":account": account },
                    |row| Ok((row.get::<_, i64>(0)? as u64, row.get(1)?, row.get(2)?)),
                )?,
                None => (1, None, None),
            };
            let mut hits = Vec::new();
            for hit in found {
                hits.push(self.hit(hit, &probes, &compiled.words)?);
            }
            threads.push(PimdirSearchThread {
                account,
                thread,
                count,
                first,
                last,
                hits,
            });
        }

        Ok(PimdirSearchThreadsOutput {
            threads,
            more,
            next,
            coverage: self.coverage(query, now, &options.clock, &zone)?,
        })
    }

    /// Forces a driver on every later answer of this handle, `None`
    /// restoring the plan's own choice: what runs each query under each
    /// driver, so a choice of plan never changes an answer.
    #[doc(hidden)]
    pub fn force_driver(&self, force: Option<PimdirPlanForce>) {
        self.force.set(force);
    }

    /// The thread of one item, `(account, seq)` (SEARCH §9): its id
    /// (`thread_of`), count, first and last date, and its live members in
    /// the account (`thread_members`) as hits, oldest first. An item no
    /// thread holds, one other than mail, is its own thread as the
    /// threaded listing answers it; `None` when no live placement of it
    /// is in the account.
    pub fn thread(
        &self,
        account: Option<&str>,
        seq: i64,
    ) -> Result<Option<PimdirSearchThread>, PimdirError> {
        let (kind, _, placements) = self.placements(account, seq)?;
        if placements.is_empty() {
            return Ok(None);
        }

        let mut thread = None;
        if kind == PimdirQueryKind::Mail.as_str() {
            for placement in &placements {
                thread = self
                    .conn
                    .query_row_cached(
                        search::THREAD_OF,
                        named_params! { ":link_id": placement.link_id.0 },
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?;
                if thread.is_some() {
                    break;
                }
            }
        }
        let account = account.map(ToString::to_string);
        let Some(thread) = thread else {
            let found = Found {
                account: account.clone(),
                seq,
                place: PimdirSearchCursor::default(),
            };
            return Ok(Some(PimdirSearchThread {
                account,
                thread: None,
                count: 1,
                first: None,
                last: None,
                hits: vec![self.hit(found, &[], &[])?],
            }));
        };

        let members: BTreeSet<i64> = rows(
            &self.conn,
            search::THREAD_MEMBERS,
            named_params! { ":thread": thread, ":account": account },
            |row| row.get::<_, i64>(1),
        )?
        .into_iter()
        .collect();
        let mut hits = Vec::new();
        for seq in members {
            let found = Found {
                account: account.clone(),
                seq,
                place: PimdirSearchCursor::default(),
            };
            let hit = self.hit(found, &[], &[])?;
            // NOTE: a member whose placements went since the last refresh
            // is not live any more.
            if !hit.placements.is_empty() {
                hits.push(hit);
            }
        }
        hits.sort_by(|a, b| (&a.sort_key, a.seq).cmp(&(&b.sort_key, b.seq)));
        let (count, first, last) = self.conn.query_row_cached(
            search::THREAD_SPAN,
            named_params! { ":thread": thread, ":account": account },
            |row| Ok((row.get::<_, i64>(0)? as u64, row.get(1)?, row.get(2)?)),
        )?;

        Ok(Some(PimdirSearchThread {
            account,
            thread: Some(thread),
            count,
            first,
            last,
            hits,
        }))
    }

    /// Every occurrence overlapping `[start, end)`, RFC 3339 instants in
    /// UTC, one each with its instants, by start, then `seq`, then
    /// collection (`occurrences_between`, SEARCH §7): what an agenda draws
    /// a window from, each item's row read from its summary.
    ///
    /// The index holds occurrences within its horizon alone; a range
    /// reaching past it lists what the horizon holds.
    pub fn occurrences_between(
        &self,
        start: &str,
        end: &str,
    ) -> Result<Vec<PimdirSearchOccurrence>, PimdirError> {
        Ok(rows(
            &self.conn,
            search::OCCURRENCES_BETWEEN,
            named_params! { ":start": start, ":end": end },
            |row| {
                Ok(PimdirSearchOccurrence {
                    collection: row.get(0)?,
                    seq: row.get(1)?,
                    start: row.get(2)?,
                    end: row.get(3)?,
                })
            },
        )?)
    }

    /// How many hits a query has, with what the count could not see, its
    /// dates resolved against `clock`. One read transaction holds the
    /// count and its coverage, so both answer one state of the store and
    /// the index.
    pub fn count(
        &self,
        query: &PimdirQuery,
        clock: &PimdirSearchClock,
    ) -> Result<PimdirSearchCountOutput, PimdirError> {
        let now = clock.now();
        let zone = clock.time_zone()?;
        let read = match self.conn.is_autocommit() {
            true => Some(self.conn.unchecked_transaction()?),
            false => None,
        };
        let count = self.counted(query, now, &zone)?;
        let coverage = self.coverage(query, now, clock, &zone)?;
        if let Some(read) = read {
            read.commit()?;
        }

        Ok(PimdirSearchCountOutput { count, coverage })
    }

    /// How many hits a query has: text terms alone over every collection,
    /// on an index holding the store as it stands, counted off their
    /// matches; another query by its smallest set read and every other
    /// term probed per hit, or every live item of its scope when no
    /// positive term has a set, each read once and marked per account
    /// when every term is one the scope implies.
    fn counted(&self, query: &PimdirQuery, now: i64, zone: &TimeZone) -> Result<u64, PimdirError> {
        let mut compiled = self.compile(query, now, zone)?;
        let current = self
            .conn
            .query_row_cached(search::INDEX_CURRENT, [], |row| row.get::<_, bool>(0))?;
        if current
            && self.force.get().is_none()
            && compiled.bitwise()
            && self.items_held()? <= BITMAP_SEQ_MAX
        {
            return self.count_bitwise(&compiled);
        }

        let bounds = self.estimate(&compiled)?;
        if self.force.get().is_none()
            && let Some(bits) = compiled.flagwise()
        {
            let items = self.items_held()?;
            if items <= BITMAP_SEQ_MAX
                && compiled.plan.scans_flags(
                    &bounds,
                    items.max(0) as u64,
                    &PimdirPlanCosts::default(),
                )
            {
                return self.count_flagwise(&compiled, &bits);
            }
        }
        let choice = compiled
            .plan
            .count(&bounds, &PimdirPlanCosts::default(), self.force.get());
        compiled.access = choice.access;
        self.read_terms(&compiled, false)?;
        let count = match &choice.driver {
            PimdirPlanDriver::Set(set) => self.run(
                &compiled.count_set(set, current),
                &compiled.values,
                &[],
                |row| row.get::<_, i64>(0),
            )?,
            _ if compiled.plan.is_scoped()
                && !compiled.plan.scope.is_one_collection()
                && self.items_held()? <= BITMAP_SEQ_MAX =>
            {
                return self.count_placed(&compiled);
            }
            _ => self.run(&compiled.count_scope(), &compiled.values, &[], |row| {
                row.get::<_, i64>(0)
            })?,
        };
        Ok(count.first().copied().unwrap_or(0).max(0) as u64)
    }

    /// The store's highest public id ([`ITEMS_HELD`](search::ITEMS_HELD)).
    fn items_held(&self) -> Result<i64, PimdirError> {
        Ok(self
            .conn
            .query_row_cached(search::ITEMS_HELD, [], |row| row.get::<_, i64>(0))?)
    }

    /// One item by the address a hit carries, read as the index reads it:
    /// its body extracted afresh, or its summary when it holds none.
    /// `None` when no live placement of it is in the account.
    pub fn item(
        &self,
        account: Option<&str>,
        seq: i64,
    ) -> Result<Option<PimdirSearchItem>, PimdirError> {
        let (kind, title, placements) = self.placements(account, seq)?;
        let Some(first) = placements.first() else {
            return Ok(None);
        };

        let body = placements
            .iter()
            .find_map(|placement| placement.object.as_ref());
        let blob = match body {
            Some(hash) if kind != PimdirQueryKind::File.as_str() => {
                self.reader.blobs().get(hash)?
            }
            _ => None,
        };
        let media_type = self
            .reader
            .collection_kind(&first.collection)?
            .unwrap_or_default();
        let (source, extract) = match blob {
            Some(blob) => (
                PimdirSearchMatched::Body,
                extract(&self.conn, &first.collection, seq, &media_type, Some(&blob))?,
            ),
            None => (
                PimdirSearchMatched::Summary,
                extract(&self.conn, &first.collection, seq, &media_type, None)?,
            ),
        };

        Ok(Some(PimdirSearchItem {
            account: account.map(ToString::to_string),
            seq,
            kind,
            title,
            placements,
            source,
            chunks: extract.chunks(),
            extract,
        }))
    }

    /// Every hit of a query in `sort` order, each with its place in it,
    /// read through the driver a count takes: what a tag stages its
    /// actions over.
    fn every(
        &self,
        query: &PimdirQuery,
        now: i64,
        zone: &TimeZone,
        sort: PimdirSearchSort,
    ) -> Result<Vec<Found>, PimdirError> {
        let mut compiled = self.compile(query, now, zone)?;
        let bounds = self.estimate(&compiled)?;
        let choice = compiled
            .plan
            .count(&bounds, &PimdirPlanCosts::default(), self.force.get());
        compiled.access = choice.access;
        self.read_terms(&compiled, sort == PimdirSearchSort::Relevance)?;
        self.whole(&compiled, &choice.driver, sort)
    }

    /// A query compiled and planned for a page of `options`, its text
    /// terms read into `temp.term_match` as the plan chose, with the
    /// driver it takes.
    fn planned(
        &self,
        query: &PimdirQuery,
        now: i64,
        zone: &TimeZone,
        options: &PimdirSearchOptions,
    ) -> Result<(Compiled, PimdirPlanDriver), PimdirError> {
        let mut compiled = self.compile(query, now, zone)?;
        let bounds = self.estimate(&compiled)?;
        let force = self.force.get();
        let extent = match options.sort.is_dated() && force.is_none() {
            true => PimdirPlanExtent {
                collections: self
                    .run(&compiled.collections(), &compiled.values, &[], |row| {
                        row.get::<_, i64>(0)
                    })?
                    .first()
                    .copied()
                    .unwrap_or(0)
                    .max(0) as u64,
                items: self.items_held()?.max(0) as u64,
            },
            false => PimdirPlanExtent::default(),
        };
        let choice = compiled.plan.page(
            options.sort,
            options.limit,
            extent,
            &bounds,
            &PimdirPlanCosts::default(),
            force,
        );
        debug!("search plan: {:?}", choice.driver);
        compiled.access = choice.access;
        compiled.bounds = bounds;
        self.read_terms(&compiled, options.sort == PimdirSearchSort::Relevance)?;
        Ok((compiled, choice.driver))
    }

    /// The positive text terms a page of `hits` hits tells the matches
    /// of, each paired with whether `temp.term_source` holds its sources:
    /// those read as such by the plan, and those whose sources cost less
    /// to read now than a full-text probe per placement of the page, a
    /// prefix expanding again in each.
    fn presented<'a>(
        &self,
        compiled: &'a Compiled,
        hits: usize,
    ) -> Result<Vec<(&'a Probe, bool)>, PimdirError> {
        let costs = PimdirPlanCosts::default();
        let mut presented = Vec::new();
        for probe in &compiled.probes {
            let read = compiled.access.get(probe.term) == Some(&PimdirPlanAccess::Sources);
            let cheap = compiled
                .bounds
                .get(probe.term)
                .copied()
                .flatten()
                .is_some_and(|bound| {
                    !bound.capped
                        && bound.rows.saturating_mul(costs.source_row)
                            < (hits as u64).saturating_mul(costs.text_probe)
                });
            if !read && cheap {
                for sql in compiled.sources(probe.term) {
                    self.run(sql, &compiled.values, &[], |_| Ok(()))?;
                }
            }
            presented.push((probe, read || cheap));
        }
        Ok(presented)
    }

    /// The first `limit` hits after `after` in `sort` order, and whether
    /// more follow, as `driver` reads them: a dated order's page cut by
    /// the statement or the walk, another's from every hit sorted here.
    fn ordered(
        &self,
        compiled: &mut Compiled,
        driver: &PimdirPlanDriver,
        after: Option<&PimdirSearchCursor>,
        sort: PimdirSearchSort,
        limit: usize,
    ) -> Result<(Vec<Found>, bool), PimdirError> {
        if !sort.is_dated() {
            let found = self.whole(compiled, driver, sort)?;
            return Ok(Self::page(found, after, sort, limit));
        }

        let need = limit.saturating_add(1);
        if let Some(mut found) = self.bitmap_page(compiled, after, sort, need)? {
            let more = found.len() > limit;
            found.truncate(limit);
            return Ok((found, more));
        }
        let mut found = match driver {
            PimdirPlanDriver::Set(set) => self.set_page(compiled, set, after, sort, need)?,
            PimdirPlanDriver::Walk { budget, fallback } => {
                match (self.walk(compiled, after, sort, need, *budget)?, fallback) {
                    (Some(found), _) => found,
                    (None, Some(set)) => {
                        debug!("search walk past its budget, reading set {set:?}");
                        let mut read = Vec::new();
                        for at in set {
                            if compiled.access[*at] != PimdirPlanAccess::Materialize {
                                compiled.access[*at] = PimdirPlanAccess::Materialize;
                                read.push(*at);
                            }
                        }
                        self.materialize(compiled, &read, false)?;
                        self.set_page(compiled, set, after, sort, need)?
                    }
                    (None, None) => Vec::new(),
                }
            }
            PimdirPlanDriver::Scope => self
                .walk(compiled, after, sort, need, None)?
                .unwrap_or_default(),
        };
        let more = found.len() > limit;
        found.truncate(limit);
        Ok((found, more))
    }

    /// Every hit `driver` reads, in `sort` order: its set's, or the
    /// scope's, each with its place.
    fn whole(
        &self,
        compiled: &Compiled,
        driver: &PimdirPlanDriver,
        sort: PimdirSearchSort,
    ) -> Result<Vec<Found>, PimdirError> {
        let sql = match driver {
            PimdirPlanDriver::Set(set) => compiled.rows(&compiled.driven(set), set, sort, None),
            _ => compiled.rows(&compiled.scoped(), &[], sort, None),
        };
        let mut found = self.run(&sql, &compiled.values, &[], |row| found(row, sort))?;
        found.sort_by(|left, right| left.place.compare(&right.place, sort));
        Ok(found)
    }

    /// The first `need` hits after `after` in a dated `sort`, read from
    /// the union of the sets of `set`, joined to the store, sorted and cut
    /// by one statement.
    fn set_page(
        &self,
        compiled: &Compiled,
        set: &[usize],
        after: Option<&PimdirSearchCursor>,
        sort: PimdirSearchSort,
        need: usize,
    ) -> Result<Vec<Found>, PimdirError> {
        let sql = compiled.rows(&compiled.driven(set), set, sort, Some(after.is_some()));
        let extra = page_params(after, need);
        self.run(&sql, &compiled.values, &extra, |row| found(row, sort))
    }

    /// The first `need` hits after `after` in a dated `sort`, walked from
    /// the scope's collections in the store's order ([`PimdirWalk`]);
    /// `None` when the walk examined more than `budget` rows first.
    fn walk(
        &self,
        compiled: &Compiled,
        after: Option<&PimdirSearchCursor>,
        sort: PimdirSearchSort,
        need: usize,
        budget: Option<u64>,
    ) -> Result<Option<Vec<Found>>, PimdirError> {
        let dated = PimdirWalk::head_bound(sort, after, false);
        let undated = PimdirWalk::head_bound(sort, after, true);
        let phase = |bound: Option<(bool, Option<&str>)>| {
            bound.map(|(ascending, key)| (ascending, key.is_some()))
        };
        let key = |bound: Option<(bool, Option<&str>)>| {
            Value::from(bound.and_then(|(_, key)| key.map(String::from)))
        };
        let extra = [
            (String::from(":dated_key"), key(dated)),
            (String::from(":undated_key"), key(undated)),
        ];
        let sql = compiled.heads(phase(dated), phase(undated));
        let collections = self.run(&sql, &compiled.values, &extra, |row| {
            Ok(PimdirWalkCollection {
                id: row.get(0)?,
                account: row.get(1)?,
                undated: row.get(2)?,
                head: row.get(3)?,
            })
        })?;

        let mut walk = PimdirWalk::new(sort, after, need, collections.clone(), budget);
        let mut arg = None;
        loop {
            match walk.resume(arg.take()) {
                PimdirCoroutineState::Yielded(step) => {
                    let collection = &collections[step.collection];
                    let account = Value::from(
                        (!collection.account.is_empty()).then(|| collection.account.clone()),
                    );
                    let (key, seq) = match &step.after {
                        Some((key, seq)) => (Value::from(key.clone()), Value::Integer(*seq)),
                        None => (Value::Null, Value::Null),
                    };
                    let extra = [
                        (
                            String::from(":collection"),
                            Value::from(collection.id.clone()),
                        ),
                        (String::from(":account"), account),
                        (
                            String::from(":akey"),
                            Value::from(collection.account.clone()),
                        ),
                        (String::from(":key"), key),
                        (String::from(":seq"), seq),
                        (String::from(":floor"), Value::from(step.floor.clone())),
                        (
                            String::from(":rows"),
                            Value::Integer(step.rows.min(i64::MAX as usize) as i64),
                        ),
                    ];
                    let rows =
                        self.run(&compiled.step(&step), &compiled.values, &extra, |row| {
                            Ok(PimdirWalkRow {
                                sort_key: row.get(0)?,
                                seq: row.get(1)?,
                                pass: row.get::<_, i64>(2)? != 0,
                            })
                        })?;
                    arg = Some(rows);
                }
                PimdirCoroutineState::Complete(PimdirWalkOutput::Page(places)) => {
                    let found = places
                        .into_iter()
                        .map(|place| Found {
                            account: (!place.account.is_empty()).then(|| place.account.clone()),
                            seq: place.seq,
                            place,
                        })
                        .collect();
                    return Ok(Some(found));
                }
                PimdirCoroutineState::Complete(PimdirWalkOutput::Exceeded) => return Ok(None),
            }
        }
    }

    /// The hits of one thread in an account (SEARCH §9), in `sort` order:
    /// its live members there (`thread_members`) meeting the query, each
    /// with its place.
    fn members(
        &self,
        compiled: &Compiled,
        thread: &str,
        account: Option<&str>,
        sort: PimdirSearchSort,
    ) -> Result<Vec<Found>, PimdirError> {
        let seqs: BTreeSet<i64> = rows(
            &self.conn,
            search::THREAD_MEMBERS,
            named_params! { ":thread": thread, ":account": account },
            |row| row.get::<_, i64>(1),
        )?
        .into_iter()
        .collect();
        if seqs.is_empty() {
            return Ok(Vec::new());
        }
        let mut extra = vec![(
            String::from(":akey"),
            Value::from(account.unwrap_or_default().to_string()),
        )];
        let mut listed = Vec::new();
        for (at, seq) in seqs.into_iter().enumerate() {
            listed.push(format!("(:m{at})"));
            extra.push((format!(":m{at}"), Value::Integer(seq)));
        }
        let hits = format!(
            "SELECT :akey AS akey, column1 AS seq FROM (VALUES {})",
            listed.join(", ")
        );
        let sql = compiled.rows(&hits, &[], sort, None);
        let mut found = self.run(&sql, &compiled.values, &extra, |row| found(row, sort))?;
        found.sort_by(|left, right| left.place.compare(&right.place, sort));
        Ok(found)
    }

    /// What the plan measures of each leaf's set, `None` for a leaf it
    /// does not.
    fn estimate(&self, compiled: &Compiled) -> Result<Vec<Option<PimdirPlanBound>>, PimdirError> {
        let mut bounds = vec![None; compiled.plan.leaves.len()];
        for at in compiled.plan.measured() {
            let Some(sql) = compiled.estimate(at) else {
                continue;
            };
            let rows = self.run(sql, &compiled.values, &[], |row| row.get::<_, i64>(0))?;
            let rows = rows.first().copied().unwrap_or(0).max(0) as u64;
            bounds[at] = Some(PimdirPlanBound {
                rows,
                capped: rows >= CAP,
            });
        }
        trace!("search estimates: {bounds:?}");
        Ok(bounds)
    }

    /// How many hits the scope's live placements hold, for a query whose
    /// every term the scope implies: each placement read once off
    /// `items_by_sort`, its hit marked in a bitmap of public ids per
    /// account, so a hit filed in several collections counts once without
    /// a seek per placement for its representative.
    fn count_placed(&self, compiled: &Compiled) -> Result<u64, PimdirError> {
        let mut accounts: BTreeMap<String, usize> = BTreeMap::new();
        let mut held: BTreeMap<i64, usize> = BTreeMap::new();
        for (rowid, account) in
            self.run(&compiled.scope_accounts(), &compiled.values, &[], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
        {
            let next = accounts.len();
            held.insert(rowid, *accounts.entry(account).or_insert(next));
        }
        let mut marked: Vec<Vec<u64>> = vec![Vec::new(); accounts.len()];
        let mut count = 0u64;
        let mut last: Option<(i64, usize)> = None;
        self.run(&compiled.scope_placements(), &compiled.values, &[], |row| {
            let rowid: i64 = row.get(0)?;
            let seq = row.get::<_, i64>(1)?.max(0) as usize;
            let account = match last {
                Some((at, account)) if at == rowid => account,
                _ => {
                    let account = held.get(&rowid).copied().unwrap_or(0);
                    last = Some((rowid, account));
                    account
                }
            };
            let Some(bits) = marked.get_mut(account) else {
                return Ok(());
            };
            let (word, bit) = (seq / 64, 1u64 << (seq % 64));
            if bits.len() <= word {
                bits.resize(word + 1, 0);
            }
            if bits[word] & bit == 0 {
                bits[word] |= bit;
                count += 1;
            }
            Ok(())
        })?;
        Ok(count)
    }

    /// How many hits a query of text terms alone has over every
    /// collection, on an index holding the store as it stands, whose
    /// placements are then the store's live ones: each term's placements
    /// read once off the full-text index into a bitmap of public ids per
    /// account, the bitmaps combined as the query combines its terms, with
    /// no seek into the store and no table of matches.
    fn count_bitwise(&self, compiled: &Compiled) -> Result<u64, PimdirError> {
        Ok(hits_counted(&self.bitwise_hits(compiled)?.0))
    }

    /// The hits of a query [`bitwise`](Compiled::bitwise) admits, as
    /// bitmaps of public ids per account, with each account's name by its
    /// number (empty for none): matched jointly when the split ids allow,
    /// else term by term.
    fn bitwise_hits(&self, compiled: &Compiled) -> Result<(Hits, Vec<String>), PimdirError> {
        let counted: i64 = self
            .conn
            .query_row_cached(search::COUNT_ACCOUNTS, [], |row| row.get(0))?;
        let mut accounts: BTreeMap<String, usize> = BTreeMap::new();
        let mut held: HashMap<String, usize> = HashMap::new();
        let names = match counted > 1 {
            true => {
                for (id, account) in self.run(search::COLLECTION_ACCOUNTS, &[], &[], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })? {
                    let next = accounts.len();
                    held.insert(id, *accounts.entry(account).or_insert(next));
                }
                let mut names = vec![String::new(); accounts.len()];
                for (name, at) in accounts {
                    names[at] = name;
                }
                names
            }
            false => self.run(search::ONE_ACCOUNT, &[], &[], |row| row.get::<_, String>(0))?,
        };
        let held = Held {
            collections: held,
            accounts: counted.max(0) as usize,
        };
        let hits = match self.marked_jointly(compiled, &held)? {
            Some(hits) => hits,
            None => self.marked(compiled, &compiled.plan.root, &held)?,
        };
        Ok((hits, names))
    }

    /// The first `need` hits after `after` in a dated `sort` of a query of
    /// text terms alone ([`bitwise`](Compiled::bitwise)) over a current
    /// index, read from its bitmaps rather than walked: its hits written
    /// to `temp.hit_page`, then joined to the store, sorted and cut by one
    /// statement, as [`set_page`](Self::set_page) cuts a set's. Only for
    /// an `and` of two positive text terms or more, one of whose sets is
    /// not capped, matching at most [`PAGE_HITS_MAX`] documents together
    /// (`JOINT_BODIES`, `JOINT_SUMMARIES`, a count stopping there, so
    /// `enron california`, about 78k hits on the 1M tier, never builds
    /// its bitmaps) and holding at most as many hits: a walk probes
    /// each row it meets against every term and meets many before an
    /// intersection's page, where one term's page or an `or`'s is met
    /// sooner than a seek per hit reads it (on the 1M tier `power
    /// california` 337 to 111 ms, while `subject:meeting` went from 38 to
    /// 105 ms and `california or texas` from 45 to 95 read this way).
    fn bitmap_page(
        &self,
        compiled: &Compiled,
        after: Option<&PimdirSearchCursor>,
        sort: PimdirSearchSort,
        need: usize,
    ) -> Result<Option<Vec<Found>>, PimdirError> {
        if self.force.get().is_some() || !compiled.bitwise() {
            return Ok(None);
        }
        let positive: Vec<u64> = compiled
            .plan
            .leaves
            .iter()
            .zip(&compiled.bounds)
            .filter(|(leaf, _)| leaf.is_text() && !leaf.negated)
            .map(|(_, bound)| bound.map_or(CAP, |bound| bound.rows))
            .collect();
        let smallest = positive.iter().copied().min().unwrap_or(CAP);
        let narrow = matches!(compiled.plan.root, PimdirPlanNode::And(_))
            && positive.len() >= 2
            && smallest < CAP;
        if !narrow || self.items_held()? > BITMAP_SEQ_MAX {
            return Ok(None);
        }
        let current = self
            .conn
            .query_row_cached(search::INDEX_CURRENT, [], |row| row.get::<_, bool>(0))?;
        if !current {
            return Ok(None);
        }
        let limit = PAGE_HITS_MAX as i64 + 1;
        let mut documents = 0i64;
        for (statement, summary) in [
            (search::JOINT_BODIES, false),
            (search::JOINT_SUMMARIES, true),
        ] {
            if let Some(expression) = compiled.joint(summary) {
                documents += self.conn.query_row_cached(
                    statement,
                    named_params! { ":match": expression, ":limit": limit },
                    |row| row.get::<_, i64>(0),
                )?;
            }
        }
        if documents > PAGE_HITS_MAX as i64 {
            return Ok(None);
        }
        let (hits, names) = self.bitwise_hits(compiled)?;
        if hits_counted(&hits) > PAGE_HITS_MAX {
            return Ok(None);
        }

        self.conn.execute_batch(search::CREATE_HIT_PAGE)?;
        self.conn.execute_cached(search::CLEAR_HIT_PAGE, [])?;
        let mut insert = self.conn.prepare_cached(search::INSERT_HIT_PAGE)?;
        for (account, bits) in hits.iter().enumerate() {
            let Some(name) = names.get(account) else {
                continue;
            };
            for (at, word) in bits.iter().enumerate() {
                let mut word = *word;
                while word != 0 {
                    let seq = (at * 64 + word.trailing_zeros() as usize) as i64;
                    insert.execute(named_params! { ":akey": name, ":seq": seq })?;
                    word &= word - 1;
                }
            }
        }
        drop(insert);

        let sql = compiled.met_rows(
            "SELECT akey, seq FROM temp.hit_page",
            sort,
            Some(after.is_some()),
        );
        let extra = page_params(after, need);
        Ok(Some(self.run(&sql, &compiled.values, &extra, |row| {
            found(row, sort)
        })?))
    }

    /// The hits of a query [`bitwise`](Compiled::bitwise) admits as bitmaps
    /// of public ids per account, its terms matched together by one FTS5
    /// expression over the bodies and one over the summaries
    /// ([`joint`](Compiled::joint)), each matching document's placements
    /// marked; then each split public id (`split_seqs`, SEARCH §3) taken
    /// away and evaluated per account, each term holding when any of its
    /// placements there matches it (SEARCH §8.2), its body by `match_object`
    /// and its summary by `match_summary`. `None` past [`SPLIT_MAX`] split
    /// ids.
    fn marked_jointly(
        &self,
        compiled: &Compiled,
        held: &Held,
    ) -> Result<Option<Hits>, PimdirError> {
        let split: i64 = self.conn.query_row_cached(
            search::SPLIT_HELD,
            named_params! { ":limit": SPLIT_MAX + 1 },
            |row| row.get(0),
        )?;
        if split > SPLIT_MAX {
            return Ok(None);
        }

        let mut marked = vec![Vec::new(); held.accounts];
        for (statement, summary) in [
            (search::MATCH_OBJECTS, false),
            (search::MATCH_SUMMARIES, true),
        ] {
            let Some(expression) = compiled.joint(summary) else {
                continue;
            };
            let sql = statement
                .replace("bm25(object_text)", "NULL")
                .replace("bm25(summary_text)", "NULL");
            let bound = [(String::from(":match"), Value::from(expression))];
            self.run(&sql, &[], &bound, |row| {
                if let Some(account) = held.account(row)? {
                    hit(&mut marked, account, row.get(1)?);
                }
                Ok(())
            })?;
        }

        let leaves = compiled.plan.leaves.len();
        let placements = self.run(search::SPLIT_SEQS, &[], &[], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        let mut split: BTreeMap<i64, Vec<(String, Option<String>)>> = BTreeMap::new();
        for (seq, collection, hash) in placements {
            split.entry(seq).or_default().push((collection, hash));
        }
        for (seq, placements) in split {
            forget(&mut marked, seq);
            let mut holding: BTreeMap<usize, Vec<bool>> = BTreeMap::new();
            for (collection, hash) in placements {
                let Some(account) = held.of(&collection) else {
                    continue;
                };
                let terms = holding
                    .entry(account)
                    .or_insert_with(|| vec![false; leaves]);
                for (at, term) in terms.iter_mut().enumerate() {
                    let Some((expression, summary)) = compiled.text_of(at) else {
                        continue;
                    };
                    let expression = Value::from(String::from(expression));
                    if let Some(hash) = &hash {
                        let probe = [
                            (String::from(":match"), expression.clone()),
                            (String::from(":hash"), Value::from(hash.clone())),
                        ];
                        *term |= !self
                            .run(search::MATCH_OBJECT, &[], &probe, |_| Ok(()))?
                            .is_empty();
                    }
                    if summary && !*term {
                        let probe = [
                            (String::from(":match"), expression),
                            (String::from(":collection"), Value::from(collection.clone())),
                            (String::from(":seq"), Value::from(seq)),
                        ];
                        *term |= !self
                            .run(search::MATCH_SUMMARY, &[], &probe, |_| Ok(()))?
                            .is_empty();
                    }
                }
            }
            for (account, terms) in holding {
                if holds(&compiled.plan.root, &terms) {
                    hit(&mut marked, account, seq);
                }
            }
        }
        Ok(Some(marked))
    }

    /// The hits of `node` as bitmaps of public ids per account, under the
    /// shapes [`bitwise`](Compiled::bitwise) admits: an `and` holding a
    /// positive operand, its negated ones taken away; an `or` of positive
    /// operands; a text term, its placements' hits.
    fn marked(
        &self,
        compiled: &Compiled,
        node: &PimdirPlanNode,
        held: &Held,
    ) -> Result<Hits, PimdirError> {
        match node {
            PimdirPlanNode::Leaf(at) => {
                let mut marked = vec![Vec::new(); held.accounts];
                for sql in compiled.placements(*at) {
                    self.run(sql, &compiled.values, &[], |row| {
                        if let Some(account) = held.account(row)? {
                            hit(&mut marked, account, row.get(1)?);
                        }
                        Ok(())
                    })?;
                }
                Ok(marked)
            }
            PimdirPlanNode::Or(operands) => {
                let mut marked = vec![Vec::new(); held.accounts];
                for operand in operands {
                    unite(&mut marked, &self.marked(compiled, operand, held)?);
                }
                Ok(marked)
            }
            PimdirPlanNode::And(operands) => {
                let mut marked: Option<Hits> = None;
                for operand in operands {
                    if matches!(operand, PimdirPlanNode::Not(_)) {
                        continue;
                    }
                    let other = self.marked(compiled, operand, held)?;
                    match &mut marked {
                        None => marked = Some(other),
                        Some(marked) => intersect(marked, &other),
                    }
                }
                let mut marked = marked.unwrap_or_else(|| vec![Vec::new(); held.accounts]);
                for operand in operands {
                    let PimdirPlanNode::Not(negated) = operand else {
                        continue;
                    };
                    if hits_counted(&marked) == 0 {
                        break;
                    }
                    subtract(&mut marked, &self.marked(compiled, negated, held)?);
                }
                Ok(marked)
            }
            PimdirPlanNode::Not(_) => Ok(vec![Vec::new(); held.accounts]),
        }
    }

    /// How many hits a query of system flags alone has over a scope of
    /// kinds and accounts, `bits` the bit each leaf tests
    /// ([`flagwise`](Compiled::flagwise)): the scope's live placements
    /// read once off `items_by_sort` with their `items.marks`, each hit
    /// marked in a bitmap per account, and per bit whether any of its
    /// placements holds it (SEARCH §8.2), the bitmaps then combined as the
    /// query combines its terms, a term the scope implies holding for
    /// every hit.
    fn count_flagwise(
        &self,
        compiled: &Compiled,
        bits: &[Option<i64>],
    ) -> Result<u64, PimdirError> {
        let mut accounts: BTreeMap<String, usize> = BTreeMap::new();
        let mut held: BTreeMap<i64, usize> = BTreeMap::new();
        for (rowid, account) in
            self.run(&compiled.scope_accounts(), &compiled.values, &[], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
        {
            let next = accounts.len();
            held.insert(rowid, *accounts.entry(account).or_insert(next));
        }
        let wanted: Vec<i64> = bits
            .iter()
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut every: Hits = vec![Vec::new(); accounts.len()];
        let mut holding: Vec<Hits> = vec![vec![Vec::new(); accounts.len()]; wanted.len()];
        let mut last: Option<(i64, usize)> = None;
        self.run(&compiled.scope_marks(), &compiled.values, &[], |row| {
            let rowid: i64 = row.get(0)?;
            let account = match last {
                Some((at, account)) if at == rowid => account,
                _ => {
                    let Some(account) = held.get(&rowid).copied() else {
                        return Ok(());
                    };
                    last = Some((rowid, account));
                    account
                }
            };
            let seq: i64 = row.get(1)?;
            let marks: i64 = row.get(2)?;
            hit(&mut every, account, seq);
            for (bit, hits) in wanted.iter().zip(holding.iter_mut()) {
                if marks & bit == *bit {
                    hit(hits, account, seq);
                }
            }
            Ok(())
        })?;
        let leaf = |at: usize| match bits[at] {
            Some(bit) => wanted
                .iter()
                .position(|wanted| *wanted == bit)
                .map_or_else(|| every.clone(), |at| holding[at].clone()),
            None => every.clone(),
        };
        Ok(hits_counted(&flagged(&compiled.plan.root, &every, &leaf)))
    }

    /// Reads what each text term needs under the plan's access: its
    /// matches into `temp.term_match`, ranked when `ranked`, or its
    /// bodies and summaries into `temp.term_source`.
    fn read_terms(&self, compiled: &Compiled, ranked: bool) -> Result<(), PimdirError> {
        self.materialize(compiled, &compiled.materialized(), ranked)?;
        for at in compiled.sourced() {
            for sql in compiled.sources(at) {
                self.run(sql, &compiled.values, &[], |_| Ok(()))?;
            }
        }
        Ok(())
    }

    /// Reads the matches of the leaves `leaves` into `temp.term_match`,
    /// ranked when `ranked`.
    fn materialize(
        &self,
        compiled: &Compiled,
        leaves: &[usize],
        ranked: bool,
    ) -> Result<(), PimdirError> {
        for at in leaves {
            for sql in compiled.matches(*at, ranked) {
                self.run(&sql, &compiled.values, &[], |_| Ok(()))?;
            }
        }
        Ok(())
    }

    /// Runs one statement of a compiled query, every `:vn` it names bound
    /// to the n-th of `values` and every other name to its value in
    /// `extra`, collecting each row through `map`. Prepared once per
    /// connection, a walk running its step again and again.
    fn run<T>(
        &self,
        sql: &str,
        values: &[Value],
        extra: &[(String, Value)],
        mut map: impl FnMut(&Row) -> rusqlite::Result<T>,
    ) -> Result<Vec<T>, PimdirError> {
        trace!("search statement: {sql}");
        let mut statement = self.conn.prepare_cached(sql).map_err(unanswerable)?;
        let names: Vec<(usize, String)> = (1..=statement.parameter_count())
            .filter_map(|at| Some((at, statement.parameter_name(at)?.to_string())))
            .collect();
        for (at, name) in names {
            let value = match name
                .strip_prefix(":v")
                .and_then(|n| n.parse::<usize>().ok())
            {
                Some(n) => values.get(n.wrapping_sub(1)).cloned(),
                None => extra
                    .iter()
                    .find(|(named, _)| *named == name)
                    .map(|(_, value)| value.clone()),
            };
            statement
                .raw_bind_parameter(at, value.unwrap_or(Value::Null))
                .map_err(unanswerable)?;
        }
        let mut rows = statement.raw_query();
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(unanswerable)? {
            out.push(map(row).map_err(unanswerable)?);
        }
        Ok(out)
    }
    /// The first `limit` hits of `found` following `after`, in `sort`
    /// order, and whether more follow: the page is selected before it is
    /// sorted, so a hit it skips costs a comparison or two and only the
    /// page is sorted.
    fn page(
        found: Vec<Found>,
        after: Option<&PimdirSearchCursor>,
        sort: PimdirSearchSort,
        limit: usize,
    ) -> (Vec<Found>, bool) {
        let order = |left: &Found, right: &Found| left.place.compare(&right.place, sort);
        let mut rest: Vec<Found> = found
            .into_iter()
            .filter(|hit| {
                after.is_none_or(|after| hit.place.compare(after, sort) == Ordering::Greater)
            })
            .collect();

        let more = rest.len() > limit;
        if more {
            rest.select_nth_unstable_by(limit, order);
            rest.truncate(limit);
        }
        rest.sort_by(order);
        (rest, more)
    }

    /// One hit, `(account, seq)`, as an answer carries it: its kind, title
    /// and placements, what `probes` matched and a snippet naming `words`.
    fn hit(
        &self,
        found: Found,
        probes: &[(&Probe, bool)],
        words: &[String],
    ) -> Result<PimdirSearchHit, PimdirError> {
        let Found { account, seq, .. } = found;
        let (kind, title, placements) = self.placements(account.as_deref(), seq)?;
        let matched = self.matched(seq, &placements, probes)?;
        let snippet = self.snippet(&kind, &placements, words)?;

        Ok(PimdirSearchHit {
            account,
            seq,
            kind,
            title,
            sort_key: placements
                .iter()
                .map(|placement| placement.sort_key.clone())
                .max()
                .unwrap_or_default(),
            placements,
            matched,
            snippet,
        })
    }

    /// What a hit's text terms matched (SEARCH §8.2): a body one of its
    /// placements holds, probed by `match_object`, else the summary of one
    /// indexed bodiless, by `match_summary`; `None` when only structured
    /// terms did. A page's hits alone are probed, never every hit. A term
    /// whose sources `temp.term_source` holds, paired with `true`, is
    /// probed there by the same keys, a seek in place of a full-text query.
    fn matched(
        &self,
        seq: i64,
        placements: &[PimdirSearchPlacement],
        probes: &[(&Probe, bool)],
    ) -> Result<Option<PimdirSearchMatched>, PimdirError> {
        let mut matched = None;
        for (probe, sourced) in probes {
            for placement in placements {
                if let Some(hash) = &placement.object {
                    let body = match sourced {
                        true => self.conn.query_row_cached(
                            search::SOURCE_OBJECT,
                            named_params! { ":term": probe.term as i64, ":hash": hash.0 },
                            |_| Ok(()),
                        ),
                        false => self.conn.query_row_cached(
                            search::MATCH_OBJECT,
                            named_params! { ":match": probe.expression, ":hash": hash.0 },
                            |_| Ok(()),
                        ),
                    }
                    .optional()
                    .map_err(unanswerable)?;
                    if body.is_some() {
                        return Ok(Some(PimdirSearchMatched::Body));
                    }
                }
                if probe.summary && matched.is_none() {
                    let summary = match sourced {
                        true => self.conn.query_row_cached(
                            search::SOURCE_SUMMARY,
                            named_params! {
                                ":term": probe.term as i64,
                                ":collection": placement.collection,
                                ":seq": seq,
                            },
                            |_| Ok(()),
                        ),
                        false => self.conn.query_row_cached(
                            search::MATCH_SUMMARY,
                            named_params! {
                                ":match": probe.expression,
                                ":collection": placement.collection,
                                ":seq": seq,
                            },
                            |_| Ok(()),
                        ),
                    }
                    .optional()
                    .map_err(unanswerable)?;
                    if summary.is_some() {
                        matched = Some(PimdirSearchMatched::Summary);
                    }
                }
            }
        }
        Ok(matched)
    }

    /// A query planned and compiled, its dates resolved against `now` in
    /// `zone`, its `linked:` ends, its `category:` values and its `label:`
    /// spellings resolved, and `temp.term_match` emptied for the matches it
    /// reads.
    fn compile(
        &self,
        query: &PimdirQuery,
        now: i64,
        zone: &TimeZone,
    ) -> Result<Compiled, PimdirError> {
        let ends = self.ends(query)?;
        let categories = self.categories(query)?;
        let labels = self.labels(query)?;
        let plan = PimdirPlan::new(query, now, zone);
        self.conn.execute_cached(search::CLEAR_TERM_MATCH, [])?;
        self.conn.execute_cached(search::CLEAR_TERM_SOURCE, [])?;
        Ok(Compiled::new(plan, &ends, &categories, &labels))
    }

    /// The labels each `label:` term of a query names (SEARCH §8, STORAGE
    /// §4.3): every spelling the store holds under
    /// [`LABEL`](property::LABEL) whose [`fold`](property::fold) is the
    /// term's, read by `list_property_values`, one seek per spelling the
    /// key holds, once per query that names a label. Both sides are folded
    /// here, in Rust, so no SQL function folds the stored side (SEARCH §5).
    fn labels(&self, query: &PimdirQuery) -> Result<Labels, PimdirError> {
        let mut labels = Vec::new();
        let mut held: Option<Vec<(String, String)>> = None;
        for (term, _) in query.terms() {
            let PimdirQueryPredicate::Label(label) = &term.predicate else {
                continue;
            };
            let held = match &mut held {
                Some(held) => held,
                None => {
                    let values: Vec<String> = rows(
                        &self.conn,
                        sql::LIST_PROPERTY_VALUES,
                        named_params! { ":key": property::LABEL },
                        |row| row.get(0),
                    )?;
                    let folded = values
                        .into_iter()
                        .map(|value| (property::fold(&value), value))
                        .collect();
                    held.insert(folded)
                }
            };
            let folded = property::fold(label);
            let spellings = held
                .iter()
                .filter(|(held, _)| *held == folded)
                .map(|(_, value)| value.clone())
                .collect();
            labels.push((folded, spellings));
        }
        Ok(labels)
    }

    /// The task categories each `category:` term of a query names (SEARCH
    /// §8): every value a task states whose fold, as STORAGE Annex A.2
    /// folds `fn`, is the term's. Both sides are folded here, in Rust, so
    /// no SQL function folds the stored side; the store's distinct values
    /// are few, read once per query that names a category.
    fn categories(&self, query: &PimdirQuery) -> Result<Categories, PimdirError> {
        let mut categories = Vec::new();
        let mut stored: Option<Vec<(String, String)>> = None;
        for (term, _) in query.terms() {
            let PimdirQueryPredicate::Category(category) = &term.predicate else {
                continue;
            };
            let stored = match &mut stored {
                Some(stored) => stored,
                None => {
                    let values: Vec<String> =
                        rows(&self.conn, search::TASK_CATEGORY_VALUES, [], |row| {
                            row.get(0)
                        })?;
                    let folded = values
                        .into_iter()
                        .map(|value| (fold::value(&value), value))
                        .collect();
                    stored.insert(folded)
                }
            };
            let folded = fold::value(category);
            let values = stored
                .iter()
                .filter(|(stored, _)| *stored == folded)
                .map(|(_, value)| value.clone())
                .collect();
            categories.push((folded, values));
        }
        Ok(categories)
    }

    /// The items each `linked:` term of a query names, every kind each is
    /// held under (SEARCH §8), as `(kind, link id)` pairs.
    fn ends(&self, query: &PimdirQuery) -> Result<Ends, PimdirError> {
        let mut ends = Vec::new();
        for (term, _) in query.terms() {
            let PimdirQueryPredicate::Linked { id, .. } = &term.predicate else {
                continue;
            };
            let named = rows(
                &self.conn,
                search::LOAD_ENDS,
                named_params! { ":seq": id.seq(), ":link_id": id.as_str() },
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            ends.push((id.clone(), named));
        }
        Ok(ends)
    }

    /// A hit's kind, title and live placements in the account.
    fn placements(
        &self,
        account: Option<&str>,
        seq: i64,
    ) -> Result<(String, String, Vec<PimdirSearchPlacement>), PimdirError> {
        let mut described = (String::new(), String::new());
        let placements = rows(
            &self.conn,
            search::HIT,
            named_params! { ":seq": seq, ":account": account },
            |row| {
                let kind: Option<String> = row.get(7)?;
                let media_type: String = row.get(6)?;
                Ok((
                    PimdirSearchPlacement {
                        collection: row.get(0)?,
                        link_id: PimdirLinkId(row.get(1)?),
                        flags: codec::flags_from_column(
                            row.get::<_, Option<String>>(2)?.as_deref(),
                        ),
                        object: row.get::<_, Option<String>>(3)?.map(PimdirHash),
                        sort_key: row.get(4)?,
                        level: codec::level_from_int(row.get(5)?),
                    },
                    kind.unwrap_or(media_type),
                    row.get::<_, String>(8)?,
                ))
            },
        )?;

        if let Some((_, kind, title)) = placements.first() {
            described = (kind.clone(), title.clone());
        }
        let placements = placements
            .into_iter()
            .map(|(placement, _, _)| placement)
            .collect();
        Ok((described.0, described.1, placements))
    }

    /// A passage of a hit's indexed body: the first chunk naming one of
    /// `words`, else the first chunk, trimmed to about [`SNIPPET_LEN`]
    /// characters around the word. A blob that cannot be read cuts none,
    /// so one hit never fails the page.
    fn snippet(
        &self,
        kind: &str,
        placements: &[PimdirSearchPlacement],
        words: &[String],
    ) -> Result<Option<String>, PimdirError> {
        let Some(hash) = placements
            .iter()
            .find_map(|placement| placement.object.as_ref())
        else {
            return Ok(None);
        };
        let chunks = rows(
            &self.conn,
            search::CHUNKS_OF,
            named_params! { ":hash": hash.0 },
            |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? as usize,
                    row.get::<_, i64>(3)? as usize,
                ))
            },
        )?;
        if chunks.is_empty() && self.own_extraction {
            return Ok(None);
        }
        let blob = match self.reader.blobs().get(hash) {
            Ok(Some(blob)) => blob,
            Ok(None) => return Ok(None),
            Err(err) => {
                debug!("cannot read body {} for a snippet: {err}", hash.0);
                return Ok(None);
            }
        };

        let media_type = match PimdirQueryKind::parse(kind) {
            Some(kind) => compile::media_type(kind),
            None => kind,
        };
        let Some(extract) = PimdirExtract::body(media_type, &blob) else {
            return Ok(None);
        };
        // NOTE: the stored offsets index the text of the indexer's own
        // extraction (SEARCH §6): under another, this one's is cut again.
        let chunks = match self.own_extraction {
            true => chunks,
            false => extract
                .chunks()
                .into_iter()
                .map(|chunk| (chunk.field.into(), chunk.range.start, chunk.range.end))
                .collect(),
        };
        let words: Vec<String> = words.iter().map(|word| fold::text(word)).collect();
        let texts: Vec<&str> = chunks
            .iter()
            .filter_map(|(field, start, end)| extract.field(field)?.get(*start..*end))
            .collect();

        for text in &texts {
            let (folded, origin) = fold_chars(text);
            if let Some(at) = words
                .iter()
                .filter(|word| !word.is_empty())
                .filter_map(|word| folded.find(word.as_str()))
                .min()
            {
                return Ok(Some(trim(text, origin[at])));
            }
        }
        Ok(texts.first().map(|text| trim(text, 0)))
    }

    /// What an answer to a query could not see (SEARCH §8.2), over the
    /// query's collections, those its conjuncts leave
    /// ([`PimdirQuery::conjuncts`]), and the date range they narrow to:
    /// per kind the four counts of the bodies out of sight (`coverage`)
    /// with what the query's terms read of them, the items the index has
    /// not folded in yet and its lag, whether the range left the horizon,
    /// for a query bounded by date the calendar items cut before its end
    /// (`cut_before`), and whether `clock` states another zone than the
    /// index's where a date term can reach a calendar item. Its dates
    /// resolve against `now` in `zone`, as the query's did.
    ///
    /// A handle answering a scope again with no commit to index.db or the
    /// store between (their `data_version`) answers what it read, reading
    /// no row of either.
    fn coverage(
        &self,
        query: &PimdirQuery,
        now: i64,
        clock: &PimdirSearchClock,
        zone: &TimeZone,
    ) -> Result<PimdirSearchCoverage, PimdirError> {
        let scope = PimdirCoverageScope::of(query, now, zone);
        let versions = (
            self.conn
                .query_row_cached(search::INDEX_DATA_VERSION, [], |r| r.get(0))?,
            self.conn
                .query_row_cached(search::STORE_DATA_VERSION, [], |r| r.get(0))?,
        );
        let cached = {
            let mut cache = self.coverages.borrow_mut();
            if cache.versions != Some(versions) {
                cache.versions = Some(versions);
                cache.held.clear();
            }
            cache.held.get(&scope).cloned()
        };
        let held = match cached {
            Some(held) => held,
            None => {
                let held = self.read_coverage(&scope)?;
                let mut cache = self.coverages.borrow_mut();
                if cache.held.len() >= COVERAGES_MAX {
                    cache.held.clear();
                }
                cache.held.insert(scope, held.clone());
                held
            }
        };

        let mut coverage = held.coverage;
        coverage.local.reads = query.reads();
        if let Some(floating) = held.floating {
            let stated = clock.zone.as_deref().unwrap_or(coverage::FLOATING_ZONE);
            coverage.zone_differs = stated != floating;
        }
        Ok(coverage)
    }

    /// The coverage of `scope` as the store and the index hold it, its
    /// `reads` and `zone_differs` left to the query.
    ///
    /// A scope holding every collection, no conjunct narrowing it or none
    /// ruling one out, has its sources and bodies out of sight rolled up
    /// over the whole store (`ROLL_COVERAGE_ALL`, the canonical `coverage_all`), no
    /// collection bound, so the cost is the rows counted, not the
    /// collections. Another reads its collections, one alone for a
    /// `collection:` conjunct, binds their ids once, every statement
    /// reading the slot, the canonical ones run as their text stands, and
    /// rolls the sources up in one statement (`ROLL_COVERAGE`). The
    /// unindexed items are counted only while the index is behind, none
    /// being stamped above its cursor otherwise.
    ///
    /// A range leaves the horizon when it starts before it or ends after
    /// it, an open side always (SEARCH §8.1), a date term under `or` or
    /// `not` counting as open on both sides. Zones are compared as names, a
    /// client stating none stating `UTC` (SEARCH §8.1).
    fn read_coverage(
        &self,
        scope: &PimdirCoverageScope,
    ) -> Result<PimdirCoverageHeld, PimdirError> {
        let named = scope.terms.iter().find_map(|term| match term {
            PimdirCoverageTerm::Collection(id) => Some(id.as_str()),
            _ => None,
        });
        let read = |r: &Row| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
            ))
        };
        let (held, all) = match (scope.terms.is_empty(), named) {
            (true, _) => (None, true),
            (false, Some(id)) => {
                let every: i64 =
                    self.conn
                        .query_row_cached(search::COUNT_COLLECTIONS, [], |r| r.get(0))?;
                let held: Vec<_> = rows(&self.conn, search::SCOPE_COLLECTION, [id], read)?
                    .into_iter()
                    .filter(|(id, account, kind)| scope.holds(id, account.as_deref(), kind))
                    .collect();
                let all = held.len() as i64 == every;
                (Some(held), all)
            }
            (false, None) => {
                let collections = rows(&self.conn, search::SCOPE_COLLECTIONS, [], read)?;
                let every = collections.len();
                let held: Vec<_> = collections
                    .into_iter()
                    .filter(|(id, account, kind)| scope.holds(id, account.as_deref(), kind))
                    .collect();
                let all = held.len() == every;
                (Some(held), all)
            }
        };
        let bound = Cell::new(false);
        let bind = || -> Result<(), PimdirError> {
            if bound.replace(true) {
                return Ok(());
            }
            match &held {
                Some(held) => {
                    bind_list(&self.conn, "collections", held.iter().map(|(id, _, _)| id))?
                }
                None => {
                    let ids: Vec<String> =
                        rows(&self.conn, search::SCOPE_COLLECTIONS, [], |r| r.get(0))?;
                    bind_list(&self.conn, "collections", ids.iter())?
                }
            }
            Ok(())
        };
        let calendar = match &held {
            Some(held) => held.iter().any(|(_, _, kind)| kind == "text/calendar"),
            None => {
                scope.dated
                    && self
                        .conn
                        .query_row_cached(search::HOLDS_CALENDARS, [], |r| r.get::<_, bool>(0))?
            }
        };

        let since = scope.since.as_deref();
        let until = scope.until.as_deref();
        let (roll, covered) = match all {
            true => (search::ROLL_COVERAGE_ALL, search::COVERAGE_ALL),
            false => {
                bind()?;
                (search::ROLL_COVERAGE, search::COVERAGE)
            }
        };
        let mut coverage = PimdirSearchCoverage {
            temporal: rolled_temporal(&self.conn, roll, since, until)?,
            ..Default::default()
        };

        let kinds = rows(&self.conn, covered, [], |r| {
            let count = |column| r.get::<_, i64>(column).map(|count| count.max(0) as u64);
            Ok((
                r.get::<_, String>(0)?,
                PimdirSearchUnread {
                    bodiless: count(1)?,
                    missing: count(2)?,
                    unparseable: count(3)?,
                    encrypted: count(4)?,
                },
            ))
        })?;
        coverage.local.kinds = kinds.into_iter().collect();
        let behind: i64 = self
            .conn
            .query_row_cached(search::BEHIND, [], |r| r.get(0))?;
        coverage.local.behind = behind.max(0) as u64;
        if behind > 0 {
            bind()?;
            let unindexed: i64 = self.conn.query_row_cached(
                search::COUNT_UNINDEXED,
                named_params! { ":since": self.cursor()? },
                |r| r.get(0),
            )?;
            coverage.local.unindexed = unindexed as u64;
        }

        if !(scope.dated && calendar) {
            return Ok(PimdirCoverageHeld {
                coverage,
                floating: None,
            });
        }

        bind()?;
        let cut: i64 = self.conn.query_row_cached(
            search::CUT_BEFORE,
            named_params! { ":end": until },
            |r| r.get(0),
        )?;
        coverage.cut = cut as u64;
        let (start, end, floating): (String, String, String) =
            self.conn
                .query_row_cached(search::LOAD_INDEX_META, [], |r| {
                    Ok((r.get(4)?, r.get(5)?, r.get(11)?))
                })?;
        coverage.horizon_left = since.is_none_or(|since| since < start.as_str())
            || until.is_none_or(|until| until > end.as_str());
        Ok(PimdirCoverageHeld {
            coverage,
            floating: Some(floating),
        })
    }
}

/// The WAL pages an indexer's connection lets grow before it checkpoints,
/// in place of SQLite's 1,000: a page a batch rewrites goes to the database
/// once per checkpoint, so fewer checkpoints write it fewer times (15% fewer
/// bytes on a first build at the default batch, a WAL of up to 64 MB).
const WAL_AUTOCHECKPOINT: i64 = 16384;

/// Opens index.db as the main database, the store attached read-only as
/// `store` through a URI (SEARCH §3): read-write and created when absent
/// for the indexer, read-only for a query client. Neither writes: the
/// indexer sets the journal mode once it found the index its own or built
/// it ([`held`]), so an index it refuses stays as it is. The indexer
/// creates an absent index.db `0600` first, its `-wal` and `-shm` taking
/// that mode (STORAGE §18).
fn connect(dir: &Path, query_only: bool) -> Result<Connection, PimdirError> {
    let access = match query_only {
        true => OpenFlags::SQLITE_OPEN_READ_ONLY,
        false => {
            modes::create_database(&dir.join("index.db"))?;
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
        }
    };
    let flags = access | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(dir.join("index.db"), flags)?;
    prepare_connection(&conn)?;
    if !query_only {
        conn.pragma_update(None, "wal_autocheckpoint", WAL_AUTOCHECKPOINT)?;
    }
    conn.execute_batch(search::CREATE_TERM_MATCH)?;

    let store = dir.join("pimdir.db").canonicalize()?;
    conn.execute_cached(
        "ATTACH DATABASE ?1 AS store",
        [format!("file:{}?mode=ro", uri_path(&store))],
    )?;
    Ok(conn)
}

/// What an index.db holds, read before an indexer folds into it or a
/// query client answers from it (SEARCH §2).
#[derive(Clone, Debug, Eq, PartialEq)]
enum PimdirIndexHeld {
    /// No schema at all: the indexer builds it.
    Empty,
    /// The canonical shape at an earlier version, or an `index_meta`
    /// naming no version: the indexer rebuilds it.
    Earlier {
        /// Its `format_version`, `None` without one.
        version: Option<String>,
        /// Its `shape`, `None` without one.
        shape: Option<String>,
    },
    /// This crate's shape and version built under another schema version,
    /// tokenizer or schema text, from another store or another copy of it,
    /// or holding a cursor above the store's last stamp: the indexer
    /// rebuilds it.
    Stale,
    /// What this crate builds, from this store, under another revision of
    /// SEARCH §6 or another indexer's own extraction (`index_meta`'s
    /// `extraction` and `extractor`, SEARCH §2): the indexer rebuilds it,
    /// which extracts every body and expands every calendar item again; a
    /// query client answers from it.
    Extracted,
    /// What this crate builds, from this store.
    Current,
}

/// What index.db holds (SEARCH §2): refused as [`PimdirError::IndexVersion`]
/// when it holds a schema and no `index_meta`, another shape, or a later or
/// unknown version, which an indexer never rebuilds, writes nor deletes;
/// else its version, then its schema version, tokenizer and schema text,
/// every table, index and trigger the schema creates compared by its
/// `sqlite_schema` text, comments removed and whitespace collapsed (the
/// shadow tables FTS5 creates, and any other table, left out), then the
/// store it was built from and the cursor it folded, against the store's
/// own (STORAGE §4.5), then the revisions of its extraction against this
/// build's.
fn held(conn: &Connection, dir: &Path) -> Result<PimdirIndexHeld, PimdirError> {
    let objects: i64 =
        conn.query_row_cached("SELECT count(*) FROM main.sqlite_schema", [], |r| r.get(0))?;
    if objects == 0 {
        return Ok(PimdirIndexHeld::Empty);
    }

    let columns: Vec<String> = rows(
        conn,
        "SELECT name FROM pragma_table_info('index_meta', 'main')",
        [],
        |r| r.get(0),
    )?;
    let column = |name: &'static str| match columns.iter().any(|column| column == name) {
        true => name,
        false => "NULL",
    };
    let named: Option<(Option<String>, Option<String>)> = match columns.is_empty() {
        true => None,
        false => conn
            .query_row_cached(
                &format!(
                    "SELECT {}, {} FROM main.index_meta WHERE id = 1",
                    column("format_version"),
                    column("shape")
                ),
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?,
    };
    let refused = |version: Option<String>, shape: Option<String>| PimdirError::IndexVersion {
        path: dir.join("index.db"),
        version,
        shape,
    };
    let Some((version, named)) = named else {
        return Err(refused(None, None));
    };
    let Some(found) = version.clone() else {
        return Ok(PimdirIndexHeld::Earlier {
            version,
            shape: named,
        });
    };
    if named.as_deref() != Some(search::SHAPE) {
        return Err(refused(version, named));
    }
    match draft(&found) {
        Some(number) if number < search::FORMAT_DRAFT => {
            return Ok(PimdirIndexHeld::Earlier {
                version,
                shape: named,
            });
        }
        Some(number) if number == search::FORMAT_DRAFT => (),
        _ => return Err(refused(version, named)),
    }

    let version: i64 = conn.pragma_query_value(Some("main"), "user_version", |r| r.get(0))?;
    if version != search::VERSION {
        return Ok(PimdirIndexHeld::Stale);
    }

    // NOTE: before `load_index_meta`, which names columns an index of an
    // earlier schema text may lack.
    let declared: BTreeMap<String, String> = shape(conn, "main")?.into_iter().collect();
    let same = SHAPE_DECLARED
        .iter()
        .all(|(name, sql)| declared.get(name).is_some_and(|held| held == sql));
    if !same {
        return Ok(PimdirIndexHeld::Stale);
    }

    type Meta = (i64, String, i64, String, i64, i64, String);
    let (version, tokenizer, change, id, incarnation, extraction, extractor): Meta = conn
        .query_row_cached(search::LOAD_INDEX_META, [], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(7)?,
                r.get(8)?,
                r.get(12)?,
                r.get(13)?,
            ))
        })?;
    if version != search::VERSION || tokenizer != search::TOKENIZER {
        return Ok(PimdirIndexHeld::Stale);
    }

    let (head, store_id, store_incarnation): (i64, String, i64) =
        conn.query_row_cached(sql::LOAD_CHANGE_CURSOR, [], |r| {
            Ok((r.get(0)?, r.get(2)?, r.get(3)?))
        })?;
    if id != store_id || incarnation != store_incarnation || change > head {
        debug!("search index was built from another store or copy of it");
        return Ok(PimdirIndexHeld::Stale);
    }
    if extraction != extract::EXTRACTION || extractor != extract::EXTRACTOR {
        debug!("search index stands under extraction {extraction} by {extractor:?}");
        return Ok(PimdirIndexHeld::Extracted);
    }

    Ok(PimdirIndexHeld::Current)
}

/// The number of a draft version of SEARCH.md, `search-draft-N`, versions
/// being ordered by it below every release (SEARCH §2); `None` for a
/// release or a name of no known form, which no draft implements.
fn draft(version: &str) -> Option<u32> {
    let number = version.strip_prefix("search-draft-")?;
    match number.bytes().all(|b| b.is_ascii_digit()) {
        true => number.parse().ok(),
        false => None,
    }
}

/// Builds index.db afresh: the files of the one there removed, the schema
/// applied and `index_meta` written (SEARCH §2).
#[cfg(feature = "indexer")]
fn build(dir: &Path) -> Result<Connection, PimdirError> {
    debug!("building search index at {}", dir.display());
    for name in ["index.db", "index.db-wal", "index.db-shm"] {
        match fs::remove_file(dir.join(name)) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err.into()),
            _ => (),
        }
    }
    let mut conn = connect(dir, false)?;
    conn.execute_batch("PRAGMA main.journal_mode = WAL;")?;
    init(&mut conn)?;
    Ok(conn)
}

/// The tables, indexes and triggers database `schema` declares, by name,
/// with their text as SEARCH §2 compares it; the shadow tables of each
/// virtual table, and the automatic indexes, left out.
fn shape(conn: &Connection, schema: &str) -> Result<Vec<(String, String)>, PimdirError> {
    let declared = rows(
        conn,
        &format!(
            "SELECT name, sql FROM {schema}.sqlite_schema \
             WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'"
        ),
        [],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )?;
    let virtuals: Vec<String> = declared
        .iter()
        .filter(|(_, sql)| {
            sql.trim_start()
                .to_ascii_uppercase()
                .starts_with("CREATE VIRTUAL TABLE")
        })
        .map(|(name, _)| format!("{name}_"))
        .collect();

    Ok(declared
        .into_iter()
        .filter(|(name, _)| !virtuals.iter().any(|prefix| name.starts_with(prefix)))
        .map(|(name, sql)| (name, schema::normalised(&sql)))
        .collect())
}

/// Applies the index schema to an empty index.db, refusing a SQLite that
/// lacks FTS5 as [`PimdirError::Fts5`].
#[cfg(feature = "indexer")]
fn init(conn: &mut Connection) -> Result<(), PimdirError> {
    let tx = conn.transaction()?;
    for migration in search::MIGRATIONS {
        if let Err(err) = tx.execute_batch(migration) {
            if err.to_string().contains("fts5") {
                return Err(PimdirError::Fts5 {
                    version: rusqlite::version(),
                });
            }
            return Err(err.into());
        }
    }
    tx.pragma_update(Some("main"), "user_version", search::VERSION)?;
    tx.execute_cached(
        search::INIT_INDEX_META,
        named_params! {
            ":version": search::VERSION,
            ":tokenizer": search::TOKENIZER,
            ":extraction": extract::EXTRACTION,
            ":extractor": extract::EXTRACTOR,
        },
    )?;
    tx.commit()?;
    Ok(())
}

/// Takes index.lock exclusively, `None` when another indexer holds it,
/// opened `0600` without following a link (STORAGE §18).
#[cfg(feature = "indexer")]
fn lock(dir: &Path) -> Result<Option<File>, PimdirError> {
    let file = lock::open(&dir.join(LOCK))?;
    // NOTE: named through the trait, as the owner lock does: std's own
    // inherent method from 1.89 would shadow it.
    match FileExt::try_lock(&file) {
        Ok(()) => Ok(Some(file)),
        Err(_) => Ok(None),
    }
}

/// The clock's now, Unix seconds.
fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64)
}

/// A path as a `file:` URI path: every byte but the unreserved ones and
/// `/` percent-encoded.
fn uri_path(path: &Path) -> String {
    let mut out = String::new();
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(byte as char)
            }
            byte => out += &format!("%{byte:02X}"),
        }
    }
    out
}

/// Extracts an item's text (SEARCH §6): a message, a card or a calendar
/// resource through its parser, an unparseable one from what its summary
/// could read; a file, and an item holding no body, from its summary and
/// addresses.
fn extract(
    conn: &Connection,
    collection: &str,
    seq: i64,
    kind: &str,
    body: Option<&[u8]>,
) -> Result<PimdirExtract, PimdirError> {
    let parsed = body.and_then(|body| PimdirExtract::body(kind, body));
    if let Some(extract) = parsed.as_ref()
        && extract.status != PimdirExtractStatus::Unparseable
    {
        return Ok(parsed.unwrap_or_default());
    }

    let (summary, addresses) = summary::summary(conn, collection, seq, kind)?;
    let mut extract = PimdirExtract::summary(summary.as_ref(), &addresses);
    if parsed.is_some() {
        extract.status = PimdirExtractStatus::Unparseable;
    }
    Ok(extract)
}

/// A text folded for a snippet's word search as the index folds it
/// (SEARCH §5), each character alone, with the character of `text` each
/// byte of the fold came from, so a word found in the fold is placed in
/// the text. A mark the fold drops maps to nothing.
fn fold_chars(text: &str) -> (String, Vec<usize>) {
    let mut folded = String::with_capacity(text.len());
    let mut origin = Vec::with_capacity(text.len());
    for (at, c) in text.chars().enumerate() {
        let mut buf = [0; 4];
        let one = fold::text(c.encode_utf8(&mut buf));
        origin.extend(core::iter::repeat_n(at, one.len()));
        folded.push_str(&one);
    }
    (folded, origin)
}

/// About [`SNIPPET_LEN`] characters of a chunk around the character at
/// `at`, whitespace runs collapsed, an ellipsis marking each cut.
fn trim(text: &str, at: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let start = at.saturating_sub(SNIPPET_LEN / 4);
    let start = match chars[..start].iter().rposition(|c| c.is_whitespace()) {
        Some(space) if start - space < 20 => space + 1,
        _ => start,
    };
    let end = (start + SNIPPET_LEN).min(chars.len());

    let mut snippet = String::new();
    if start > 0 {
        snippet.push('…');
    }
    let mut space = false;
    for c in &chars[start..end] {
        if c.is_whitespace() {
            space = true;
            continue;
        }
        if space && !snippet.is_empty() && !snippet.ends_with('…') {
            snippet.push(' ');
        }
        space = false;
        snippet.push(*c);
    }
    if end < chars.len() {
        snippet.push('…');
    }
    snippet
}

/// A hit as [`Compiled::rows`] answers it, its place in `sort`: the
/// `title` and `sender` orders fold its title as STORAGE Annex A.2 folds
/// `fn`, which SQL cannot, so a page's cursor compares in the very order
/// the answer is in.
fn found(row: &Row, sort: PimdirSearchSort) -> rusqlite::Result<Found> {
    let akey: String = row.get(0)?;
    let title: String = row.get(5)?;
    let place = PimdirSearchCursor {
        undated: row.get::<_, Option<bool>>(3)?.unwrap_or_default(),
        sort_key: row.get(2)?,
        rank: row.get::<_, Option<f64>>(4)?.map(f64::to_bits),
        title: match sort {
            PimdirSearchSort::Title | PimdirSearchSort::Sender => fold::value(&title),
            _ => String::new(),
        },
        seq: row.get(1)?,
        account: akey.clone(),
    };
    Ok(Found {
        account: (!akey.is_empty()).then_some(akey),
        seq: place.seq,
        place,
    })
}

/// Maps a failure of a compiled query's statement: a busy store stays
/// [`PimdirError::Busy`], and anything else is
/// [`PimdirError::Unanswerable`], never a raw SQL error, since the
/// grammar accepted the query.
fn unanswerable(err: rusqlite::Error) -> PimdirError {
    match busy_or_sql(err) {
        PimdirError::Sql(err) => PimdirError::Unanswerable(err),
        err => err,
    }
}

#[cfg(test)]
mod tests {
    use crate::client::search::*;

    /// A second answer over a scope, with no commit to the store or the
    /// index between, is the coverage the handle holds, here marked by a
    /// cut no statement answers; a commit to the store, or a refresh, has
    /// it read again (SEARCH §8.2).
    #[cfg(feature = "indexer")]
    #[test]
    fn a_coverage_is_read_again_once_the_store_or_the_index_moved() {
        use crate::client::PimdirStore;

        let dir = tempfile::tempdir().unwrap();
        let collection = |name: &str| {
            PimdirStore::open(dir.path())
                .unwrap()
                .for_source("local")
                .ensure_collection(name, "message/rfc822")
                .unwrap();
        };
        collection("INBOX");
        let mut index = PimdirIndex::open(dir.path()).unwrap();
        index
            .refresh(&PimdirIndexRefreshOptions::default())
            .unwrap();

        let clock = PimdirSearchClock::default();
        let mark = |index: &PimdirIndex| {
            for held in index.coverages.borrow_mut().held.values_mut() {
                held.coverage.cut = 7;
            }
        };
        let cut = |index: &PimdirIndex, query: &str| {
            let query = PimdirQuery::parse(query).unwrap();
            index.count(&query, &clock).unwrap().coverage.cut
        };

        for (query, added) in [("kind:mail", "Archive"), ("anything", "Sent")] {
            assert_eq!(cut(&index, query), 0, "{query}");
            mark(&index);
            assert_eq!(cut(&index, query), 7, "{query}: the coverage held");

            collection(added);
            assert_eq!(cut(&index, query), 0, "{query}: read after a commit");

            mark(&index);
            index
                .refresh(&PimdirIndexRefreshOptions::default())
                .unwrap();
            assert_eq!(cut(&index, query), 0, "{query}: read after a refresh");
        }
    }

    /// A page selected then sorted is the slice of the whole answer sorted
    /// after the cursor, in every order, at every cursor and limit, ties
    /// on every key but the public id included.
    #[test]
    fn a_page_is_the_slice_of_the_sorted_answer_after_its_cursor() {
        let found: Vec<Found> = (0..40i64)
            .map(|n| {
                let seq = (n * 17) % 40;
                Found {
                    account: None,
                    seq,
                    place: PimdirSearchCursor {
                        undated: n % 7 == 0,
                        sort_key: (n % 5 != 0).then(|| format!("{:02}", n % 3)),
                        rank: (n % 4 != 0).then_some((n % 6) as u64),
                        title: format!("t{}", n % 4),
                        seq,
                        account: String::new(),
                    },
                }
            })
            .collect();

        for sort in [
            PimdirSearchSort::NewestFirst,
            PimdirSearchSort::OldestFirst,
            PimdirSearchSort::Relevance,
            PimdirSearchSort::Title,
            PimdirSearchSort::Sender,
        ] {
            let mut sorted = found.clone();
            sorted.sort_by(|left, right| left.place.compare(&right.place, sort));
            let cursors = core::iter::once(None).chain(sorted.iter().map(|hit| Some(&hit.place)));
            for (at, after) in cursors.enumerate() {
                for limit in [0, 1, 3, 40] {
                    let (page, more) = PimdirIndex::page(found.clone(), after, sort, limit);
                    let rest = &sorted[at..];
                    let want: Vec<i64> = rest.iter().take(limit).map(|hit| hit.seq).collect();
                    let got: Vec<i64> = page.iter().map(|hit| hit.seq).collect();
                    assert_eq!(got, want, "{sort:?} after {at} by {limit}");
                    assert_eq!(more, rest.len() > limit, "{sort:?} after {at} by {limit}");
                }
            }
        }
    }
}
