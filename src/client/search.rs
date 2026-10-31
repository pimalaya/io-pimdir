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
mod refresh;
mod tag;

use alloc::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use fs4::FileExt;
use log::{debug, trace};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, TransactionBehavior, named_params, params_from_iter,
};

use crate::{
    client::{
        PimdirError, bind_list, busy_or_sql, prepare_connection,
        reader::PimdirReader,
        rows, schema,
        schema::StoreShape,
        search::{
            compile::{Categories, Compiled, Ends},
            refresh::Statements,
        },
    },
    codec,
    coroutine::PimdirCoroutineState,
    object::PimdirHash,
    placement::{PimdirFlags, PimdirLevel, PimdirLinkId},
    search::{
        coverage::PimdirSearchCoverage,
        extract::{PimdirChunk, PimdirExtract, PimdirExtractStatus},
        fold,
        query::{PimdirQuery, PimdirQueryKind, PimdirQueryPredicate},
        refresh::PimdirIndexRefresh,
    },
    sql::search,
};

/// The file a refresh locks exclusively (SEARCH §3).
const LOCK: &str = "index.lock";

/// The SQLite version the index requires, for `contentless_delete`
/// (SEARCH §2), as `sqlite3_libversion_number` spells it.
const SQLITE_MIN: i32 = 3_043_000;

/// About how many characters a snippet holds.
const SNIPPET_LEN: usize = 200;

/// A store's search index: the indexer and the query client.
pub struct PimdirIndex {
    conn: Connection,
    reader: PimdirReader,
    dir: PathBuf,
    /// Opened by [`open_query`](PimdirIndex::open_query): index.db read
    /// only, never refreshed.
    query_only: bool,
    /// Whether the attached store held every canonical table and column
    /// when opened, else every statement reading it is read as its
    /// reconciliation would leave it (STORAGE §6).
    shape: StoreShape,
}

/// How far one refresh goes.
#[derive(Clone, Copy, Debug)]
pub struct PimdirIndexRefreshOptions {
    /// The most feed rows one transaction folds, so the store's WAL still
    /// checkpoints between them (SEARCH §4).
    pub batch: usize,
    /// When to stop: checked between batches, so the pass ends at the
    /// first batch boundary past it with the cursor at that boundary.
    /// `None` folds the whole feed.
    pub deadline: Option<Instant>,
    /// The instant the horizon is drawn around (SEARCH §7), Unix seconds;
    /// `None` for the clock's.
    pub now: Option<i64>,
}

impl Default for PimdirIndexRefreshOptions {
    fn default() -> Self {
        Self {
            batch: 256,
            deadline: None,
            now: None,
        }
    }
}

/// What one refresh did.
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

impl PimdirIndexReport {
    /// Whether the index has folded every change the store had when the
    /// pass began.
    pub fn is_current(&self) -> bool {
        !self.busy && self.cursor >= self.head
    }
}

/// The order of a hit list (SEARCH §8.2).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PimdirSearchSort {
    /// `sort_key` descending, contacts last.
    #[default]
    Date,
    /// The title ascending, case folded.
    Name,
    /// FTS5 `bm25`, best first; hits no text term ranks after, by date.
    Relevance,
}

/// One page of a search.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PimdirSearchOptions {
    /// The order.
    pub sort: PimdirSearchSort,
    /// The most hits answered.
    pub limit: usize,
    /// The hits skipped before the first answered.
    pub offset: usize,
    /// The instant relative dates resolve against.
    pub clock: PimdirSearchClock,
}

impl Default for PimdirSearchOptions {
    fn default() -> Self {
        Self {
            sort: PimdirSearchSort::Date,
            limit: 50,
            offset: 0,
            clock: PimdirSearchClock::default(),
        }
    }
}

/// What a query's relative dates (`today`, `yesterday`, `2w`) resolve
/// against (SEARCH §8.1). Calendar spans are read in UTC.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PimdirSearchClock {
    /// The query's now, Unix seconds; `None` for the clock's.
    pub now: Option<i64>,
}

impl PimdirSearchClock {
    /// The query's now, Unix seconds.
    pub fn now(&self) -> i64 {
        self.now.unwrap_or_else(unix_now)
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
    /// body or a chunk.
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
struct Found {
    account: Option<String>,
    seq: i64,
    /// What it matched: `body`, `summary`, or `None` when only
    /// structured terms did.
    matched: Option<String>,
}

impl PimdirIndex {
    /// Opens the index of the store rooted at `dir`, creating index.db
    /// when absent and rebuilding it when its version, tokenizer or shape is
    /// not this build's (SEARCH §2).
    ///
    /// The store must exist ([`PimdirError::Uncreated`]). A SQLite without
    /// FTS5 or older than 3.43 is [`PimdirError::Fts5`]; another indexer
    /// holding index.lock while this one has to build is
    /// [`PimdirError::Indexing`].
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
        if !is_current(&conn)? {
            let Some(_lock) = lock(dir)? else {
                return Err(PimdirError::Indexing(dir.to_path_buf()));
            };
            // NOTE: another indexer may have built it between the check
            // and the lock.
            if !is_current(&conn)? {
                debug!("building search index at {}", dir.display());
                drop(conn);
                for name in ["index.db", "index.db-wal", "index.db-shm"] {
                    match fs::remove_file(dir.join(name)) {
                        Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err.into()),
                        _ => (),
                    }
                }
                conn = connect(dir, false)?;
                init(&mut conn)?;
            }
        }

        let shape = StoreShape::read(&conn, "store")?;
        Ok(Self {
            conn,
            reader,
            shape,
            dir: dir.to_path_buf(),
            query_only: false,
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
    /// [`PimdirError::IndexOutdated`]: either way, run the indexer.
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
        let version: i64 = conn.pragma_query_value(Some("main"), "user_version", |r| r.get(0))?;
        if version == 0 {
            return Err(PimdirError::IndexMissing(path));
        }
        if !is_current(&conn)? {
            return Err(PimdirError::IndexOutdated(path));
        }

        let shape = StoreShape::read(&conn, "store")?;
        Ok(Self {
            conn,
            reader,
            shape,
            dir: dir.to_path_buf(),
            query_only: true,
        })
    }

    /// A statement reading the attached store as the store answers it
    /// (STORAGE §6): a table or column it lacks read as its reconciliation
    /// would leave it, the statement itself on a store holding them all.
    fn store_sql<'a>(&self, statement: &'a str) -> Result<Cow<'a, str>, PimdirError> {
        self.shape.sql(&self.conn, statement)
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
            .query_row(search::LOAD_INDEX_META, [], |row| row.get(2))?)
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
    /// count or a collection stamp moved, meets again the bodies an earlier
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
    pub fn refresh(
        &mut self,
        options: &PimdirIndexRefreshOptions,
    ) -> Result<PimdirIndexReport, PimdirError> {
        if self.query_only {
            return Err(PimdirError::IndexQueryOnly(self.dir.join("index.db")));
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
        debug!(
            "refreshing search index from stamp {} to {}",
            report.cursor, report.head
        );

        let batch = options.batch.max(1);
        let blobs = self.reader.blobs();
        let now = options.now.unwrap_or_else(unix_now);
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
                shape: self.shape,
                blobs: &blobs,
            };
            let mut run = PimdirIndexRefresh::new(now, batch);
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

    /// One page of the hits of a query (SEARCH §8.2), with what it could
    /// not see. Answers from the index as it stands: refresh first.
    pub fn search(
        &self,
        query: &PimdirQuery,
        options: &PimdirSearchOptions,
    ) -> Result<PimdirSearchOutput, PimdirError> {
        let now = options.clock.now();
        let mut compiled = self.compile(query, now)?;
        // NOTE: one more than asked, so the page knows whether it is the
        // last without a count.
        let probe = options.limit.saturating_add(1);
        let mut found = self.found(&mut compiled, options.sort, Some((probe, options.offset)))?;
        let more = found.len() > options.limit;
        found.truncate(options.limit);

        let mut hits = Vec::new();
        for hit in found {
            hits.push(self.hit(hit, &compiled.words)?);
        }

        Ok(PimdirSearchOutput {
            hits,
            more,
            coverage: self.coverage(query, now)?,
        })
    }

    /// One page of the threaded listing of a query (SEARCH §9): its hits
    /// collapsed to their thread in their account, each thread with its
    /// count, first and last date, the threads in the order of their
    /// first hit and paged by `options.limit` and `options.offset`. A hit
    /// no thread holds, an item other than mail, stands alone.
    pub fn threads(
        &self,
        query: &PimdirQuery,
        options: &PimdirSearchOptions,
    ) -> Result<PimdirSearchThreadsOutput, PimdirError> {
        let now = options.clock.now();
        let mut compiled = self.compile(query, now)?;
        let found = self.found(&mut compiled, options.sort, None)?;

        let mut groups: Vec<(Option<String>, Vec<Found>)> = Vec::new();
        let mut known: BTreeMap<(Option<String>, String), usize> = BTreeMap::new();
        for hit in found {
            let thread: Option<String> = self
                .conn
                .query_row(
                    &self.store_sql(search::HIT_THREAD)?,
                    named_params! { ":seq": hit.seq, ":account": hit.account },
                    |row| row.get(0),
                )
                .optional()?;
            let Some(thread) = thread else {
                groups.push((None, vec![hit]));
                continue;
            };
            let key = (hit.account.clone(), thread.clone());
            match known.get(&key) {
                Some(at) => groups[*at].1.push(hit),
                None => {
                    known.insert(key, groups.len());
                    groups.push((Some(thread), vec![hit]));
                }
            }
        }
        let more = groups.len() > options.offset.saturating_add(options.limit);

        let mut threads = Vec::new();
        for (thread, found) in groups.into_iter().skip(options.offset).take(options.limit) {
            let account = found[0].account.clone();
            let (count, first, last) = match &thread {
                Some(thread) => self.conn.query_row(
                    &self.store_sql(search::THREAD_SPAN)?,
                    named_params! { ":thread": thread, ":account": account },
                    |row| Ok((row.get::<_, i64>(0)? as u64, row.get(1)?, row.get(2)?)),
                )?,
                None => (1, None, None),
            };
            let mut hits = Vec::new();
            for hit in found {
                hits.push(self.hit(hit, &compiled.words)?);
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
            coverage: self.coverage(query, now)?,
        })
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
                    .query_row(
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
                matched: None,
            };
            return Ok(Some(PimdirSearchThread {
                account,
                thread: None,
                count: 1,
                first: None,
                last: None,
                hits: vec![self.hit(found, &[])?],
            }));
        };

        let members: BTreeSet<i64> = rows(
            &self.conn,
            &self.store_sql(search::THREAD_MEMBERS)?,
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
                matched: None,
            };
            let hit = self.hit(found, &[])?;
            // NOTE: a member whose placements went since the last refresh
            // is not live any more.
            if !hit.placements.is_empty() {
                hits.push(hit);
            }
        }
        hits.sort_by(|a, b| (&a.sort_key, a.seq).cmp(&(&b.sort_key, b.seq)));
        let (count, first, last) = self.conn.query_row(
            &self.store_sql(search::THREAD_SPAN)?,
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
    /// relative dates resolved against `clock`.
    pub fn count(
        &self,
        query: &PimdirQuery,
        clock: &PimdirSearchClock,
    ) -> Result<PimdirSearchCountOutput, PimdirError> {
        let now = clock.now();
        let compiled = self.compile(query, now)?;
        let count: i64 = self
            .conn
            .query_row(
                &self.store_sql(&compiled.count())?,
                params_from_iter(compiled.params.iter()),
                |row| row.get(0),
            )
            .map_err(unanswerable)?;

        Ok(PimdirSearchCountOutput {
            count: count as u64,
            coverage: self.coverage(query, now)?,
        })
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
                extract(
                    &self.conn,
                    self.shape,
                    &first.collection,
                    seq,
                    &media_type,
                    Some(&blob),
                )?,
            ),
            None => (
                PimdirSearchMatched::Summary,
                extract(
                    &self.conn,
                    self.shape,
                    &first.collection,
                    seq,
                    &media_type,
                    None,
                )?,
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

    /// The hits of a compiled query in `sort` order, `page` holding how
    /// many after how many, `None` for them all.
    fn found(
        &self,
        compiled: &mut Compiled,
        sort: PimdirSearchSort,
        page: Option<(usize, usize)>,
    ) -> Result<Vec<Found>, PimdirError> {
        let sql = compiled.select(sort, page.filter(|_| sort != PimdirSearchSort::Name));
        let sql = self.store_sql(&sql)?;
        trace!("search statement: {sql}");

        let mut found = rows(
            &self.conn,
            &sql,
            params_from_iter(compiled.params.iter()),
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )
        .map_err(unanswerable)?;
        if sort == PimdirSearchSort::Name {
            // NOTE: the title folded as STORAGE Annex A.2 folds `fn`,
            // ties to the seq then the account, as the SQL order gave.
            found.sort_by_cached_key(|(_, _, _, title)| fold::value(title));
            if let Some((limit, offset)) = page {
                found = found.into_iter().skip(offset).take(limit).collect();
            }
        }

        Ok(found
            .into_iter()
            .map(|(account, seq, matched, _)| Found {
                account,
                seq,
                matched,
            })
            .collect())
    }

    /// One hit, `(account, seq)`, as an answer carries it: its kind, title
    /// and placements, what it matched and a snippet naming `words`.
    fn hit(&self, found: Found, words: &[String]) -> Result<PimdirSearchHit, PimdirError> {
        let Found {
            account,
            seq,
            matched,
        } = found;
        let (kind, title, placements) = self.placements(account.as_deref(), seq)?;
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
            matched: match matched.as_deref() {
                Some("body") => Some(PimdirSearchMatched::Body),
                Some("summary") => Some(PimdirSearchMatched::Summary),
                _ => None,
            },
            snippet,
        })
    }

    /// A query compiled with its `linked:` ends and its `category:` values
    /// resolved; what it reads of the store is read as the store answers it
    /// when it runs ([`store_sql`](Self::store_sql)).
    fn compile(&self, query: &PimdirQuery, now: i64) -> Result<Compiled, PimdirError> {
        let ends = self.ends(query)?;
        let categories = self.categories(query)?;
        Ok(Compiled::new(query, ends, categories, now))
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
                    let values: Vec<String> = rows(
                        &self.conn,
                        &self.store_sql(search::TASK_CATEGORY_VALUES)?,
                        [],
                        |row| row.get(0),
                    )?;
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
                &self.store_sql(search::LOAD_ENDS)?,
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
            &self.store_sql(search::LOAD_HIT_PLACEMENTS)?,
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
    /// characters around the word.
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
        if chunks.is_empty() {
            return Ok(None);
        }
        let Some(blob) = self.reader.blobs().get(hash)? else {
            return Ok(None);
        };

        let media_type = match PimdirQueryKind::parse(kind) {
            Some(kind) => compile::media_type(kind),
            None => kind,
        };
        let Some(extract) = PimdirExtract::body(media_type, &blob) else {
            return Ok(None);
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

    /// What an answer to a query could not see: the coverage of the
    /// collections its top-level conjunction narrows to, over the date
    /// range it narrows to, and the live items there the index has not
    /// folded in yet.
    fn coverage(&self, query: &PimdirQuery, now: i64) -> Result<PimdirSearchCoverage, PimdirError> {
        let conjuncts = match query {
            PimdirQuery::And(operands) => operands.iter().collect(),
            query => vec![query],
        };

        let mut collections = self.reader.list_collections()?;
        let mut since: Option<String> = None;
        let mut until: Option<String> = None;
        for conjunct in conjuncts {
            let PimdirQuery::Term(term) = conjunct else {
                continue;
            };
            match &term.predicate {
                PimdirQueryPredicate::Account(account) => {
                    collections.retain(|c| c.account.as_deref() == Some(account.as_str()))
                }
                PimdirQueryPredicate::Collection(id) => collections.retain(|c| &c.id == id),
                PimdirQueryPredicate::Kind(kind) => {
                    collections.retain(|c| c.kind == compile::media_type(*kind))
                }
                PimdirQueryPredicate::Date(range) => {
                    let (start, end) = compile::bounds(range, now);
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

        let ids: Vec<&str> = collections.iter().map(|c| c.id.as_str()).collect();
        let mut coverage = self
            .reader
            .search_coverage(&ids, since.as_deref(), until.as_deref())?;

        bind_list(&self.conn, "collections", &ids)?;
        let unindexed: i64 = self.conn.query_row(
            &self.store_sql(search::COUNT_UNINDEXED)?,
            named_params! { ":since": self.cursor()? },
            |r| r.get(0),
        )?;
        coverage.local.unindexed = unindexed as u64;
        let behind: i64 = self
            .conn
            .query_row(&self.store_sql(search::BEHIND)?, [], |r| r.get(0))?;
        coverage.local.behind = behind.max(0) as u64;

        // NOTE: a range leaves the horizon when it starts before it or ends
        // after it, an open side always (SEARCH §8.1); a date term under
        // `or` or `not` counts as open on both sides.
        let dated = query
            .terms()
            .into_iter()
            .any(|(term, _)| matches!(term.predicate, PimdirQueryPredicate::Date(_)));
        let calendar = collections.iter().any(|c| c.kind == "text/calendar");
        let (start, end): (String, String) =
            self.conn
                .query_row(search::LOAD_INDEX_META, [], |r| Ok((r.get(4)?, r.get(5)?)))?;
        coverage.horizon_left = dated
            && calendar
            && (since
                .as_ref()
                .is_none_or(|since| since.as_str() < start.as_str())
                || until
                    .as_ref()
                    .is_none_or(|until| until.as_str() > end.as_str()));
        Ok(coverage)
    }
}

/// Opens index.db as the main database, the store attached read-only as
/// `store` through a URI (SEARCH §3): read-write and created when absent
/// for the indexer, read-only for a query client, which writes nothing,
/// the journal mode included.
fn connect(dir: &Path, query_only: bool) -> Result<Connection, PimdirError> {
    let access = match query_only {
        true => OpenFlags::SQLITE_OPEN_READ_ONLY,
        false => OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    };
    let flags = access | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(dir.join("index.db"), flags)?;
    prepare_connection(&conn)?;
    if !query_only {
        conn.execute_batch("PRAGMA journal_mode = WAL;")?;
    }

    let store = dir.join("pimdir.db").canonicalize()?;
    conn.execute(
        "ATTACH DATABASE ?1 AS store",
        [format!("file:{}?mode=ro", uri_path(&store))],
    )?;
    Ok(conn)
}

/// Whether index.db holds this build's schema under this build's
/// tokenizer (SEARCH §2): its version, its `index_meta`, and its shape,
/// every table, index and trigger the schema creates compared by its
/// `sqlite_schema` text, comments removed and whitespace collapsed. The
/// shadow tables FTS5 creates, and any other table, are not compared.
fn is_current(conn: &Connection) -> Result<bool, PimdirError> {
    let version: i64 = conn.pragma_query_value(Some("main"), "user_version", |r| r.get(0))?;
    if version != search::VERSION {
        return Ok(false);
    }

    let meta: Option<(i64, String)> = conn
        .query_row(search::LOAD_INDEX_META, [], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()
        // NOTE: an index_meta of another shape fails the canonical read,
        // which is an index to rebuild rather than an error.
        .or_else(|err| match err {
            rusqlite::Error::SqliteFailure(..) => Ok(None),
            err => Err(err),
        })?;
    if !meta.is_some_and(|(version, tokenizer)| {
        version == search::VERSION && tokenizer == search::TOKENIZER
    }) {
        return Ok(false);
    }

    let canonical = Connection::open_in_memory()?;
    for migration in search::MIGRATIONS {
        canonical.execute_batch(migration)?;
    }
    let wanted = shape(&canonical, "main")?;
    let held: BTreeMap<String, String> = shape(conn, "main")?.into_iter().collect();
    Ok(wanted
        .into_iter()
        .all(|(name, sql)| held.get(&name).is_some_and(|held| *held == sql)))
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
    tx.execute(
        search::INIT_INDEX_META,
        named_params! { ":version": search::VERSION, ":tokenizer": search::TOKENIZER },
    )?;
    tx.commit()?;
    Ok(())
}

/// Takes index.lock exclusively, `None` when another indexer holds it.
fn lock(dir: &Path) -> Result<Option<File>, PimdirError> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(LOCK))?;
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
    shape: StoreShape,
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

    let found = refresh::summary(conn, shape, collection, seq, kind)?;
    let mut extract = PimdirExtract::summary(found.summary.as_ref(), &found.addresses);
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
