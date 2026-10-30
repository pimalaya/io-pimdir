//! The search verbs over the store's index (pimdir SEARCH.md): `index`,
//! `search`, `count`, `show`, `thread` and `tag`, the one that writes,
//! through the queue.
//!
//! They read content as SEARCH §6 extracts it, which is the spec's rule
//! and not this tool's opinion, and never render it: a hit prints as its
//! address, `(account, seq)`, its placements, what it matched and a
//! passage of its text. `search` and `count` refresh the index first,
//! within `--refresh-timeout`, and say in their coverage what they could
//! not fold in. A refused query prints its refusal object (SEARCH §8.4)
//! and exits with status 1.

use std::{
    collections::BTreeMap,
    fmt, process,
    time::{Duration, Instant},
};

use anyhow::Result;
use clap::{ArgGroup, Args, ValueEnum};
use io_pimdir::{
    client::search::{
        PimdirIndex, PimdirIndexRefreshOptions, PimdirSearchClock, PimdirSearchHit,
        PimdirSearchMatched, PimdirSearchOptions, PimdirSearchPlacement, PimdirSearchSort,
        PimdirSearchThread, tag::PimdirSearchTagSource,
    },
    placement::PimdirLevel,
    search::{
        coverage::{PimdirSearchCoverage, PimdirSearchRemote, PimdirSearchTemporal},
        query::{PimdirQuery, PimdirQueryError},
    },
};
use pimalaya_cli::printer::Printer;
use schemars::JsonSchema;
use serde::Serialize;

use crate::cli::{StoreFlags, or_dash, report};

/// The default page size of a search.
const DEFAULT_LIMIT: usize = 50;

/// Fold the store's changes into its search index.
///
/// Creates index.db beside the store when absent, rebuilds it when it was
/// built by another version, and folds the whole change feed in. Reads the
/// store as a reader, so it runs beside a sync; another indexer running
/// makes it report so and do nothing.
#[derive(Debug, Args)]
pub struct IndexCommand;

impl IndexCommand {
    /// Runs a full refresh.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        store.ensure_store()?;
        let mut index = PimdirIndex::open(store.dir()).map_err(report)?;
        let report = index
            .refresh(&PimdirIndexRefreshOptions::default())
            .map_err(report)?;

        printer.out(IndexOutput {
            folded: report.folded,
            indexed: report.indexed,
            removed: report.removed,
            reconciled: report.reconciled,
            head: report.head,
            cursor: report.cursor,
            busy: report.busy,
            current: report.is_current(),
        })
    }
}

/// Search the store's items with a query.
///
/// The query language is pimdir SEARCH.md §8: words and "phrases",
/// `field:value` terms (`from:`, `subject:`, `tag:`, `kind:`, `date:` and
/// the others), `and`, `or`, `not` or a leading `-`, and parentheses. Pass
/// it as one argument. An unknown field or a malformed query is refused
/// with where it is wrong and what would be accepted. Every hit is an
/// item in one account, addressed as `SEQ --account ACCOUNT`, which
/// `show` takes as is.
#[derive(Debug, Args)]
pub struct SearchCommand {
    /// The query, as one argument.
    #[arg(value_name = "QUERY")]
    pub query: String,

    /// The order of the hits.
    #[arg(long, value_enum, default_value_t = SortArg::Date)]
    pub sort: SortArg,

    /// Maximum number of hits to print.
    #[arg(long, short = 'n', value_name = "COUNT", default_value_t = DEFAULT_LIMIT)]
    pub limit: usize,

    /// Skip this many hits first, as the previous page's `nextOffset`.
    #[arg(long, value_name = "COUNT", default_value_t = 0)]
    pub offset: usize,

    /// Collapse the hits to their mail threads.
    ///
    /// Prints one entry per thread in each account, with its id (which
    /// `thread:` takes), how many messages it holds there, its first and
    /// last date and its hits; an item other than mail stands alone.
    /// `--limit` and `--offset` then count threads.
    #[arg(long)]
    pub threads: bool,

    #[command(flatten)]
    pub refresh: RefreshFlags,
}

impl SearchCommand {
    /// Prints one page of hits with the coverage.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let query = parse(printer, &self.query)?;
        let index = self.refresh.index(store)?;
        let limit = self.limit.max(1);
        let options = PimdirSearchOptions {
            sort: self.sort.into(),
            limit,
            offset: self.offset,
            clock: PimdirSearchClock::default(),
        };

        if self.threads {
            let answer = index.threads(&query, &options).map_err(report)?;

            return printer.out(SearchThreadsOutput {
                query: self.query,
                next_offset: answer.more.then_some(self.offset + answer.threads.len()),
                threads: answer.threads.iter().map(ThreadRow::from).collect(),
                coverage: CoverageRow::from(&answer.coverage),
            });
        }

        let answer = index.search(&query, &options).map_err(report)?;

        printer.out(SearchOutput {
            query: self.query,
            next_offset: answer.more.then_some(self.offset + answer.hits.len()),
            hits: answer.hits.iter().map(HitRow::from).collect(),
            coverage: CoverageRow::from(&answer.coverage),
        })
    }
}

/// Count the items a query matches.
///
/// Takes the query language `search` takes, and answers how many hits it
/// has with what the count could not see.
#[derive(Debug, Args)]
pub struct CountCommand {
    /// The query, as one argument.
    #[arg(value_name = "QUERY")]
    pub query: String,

    #[command(flatten)]
    pub refresh: RefreshFlags,
}

impl CountCommand {
    /// Prints the count with the coverage.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let query = parse(printer, &self.query)?;
        let index = self.refresh.index(store)?;

        let answer = index
            .count(&query, &PimdirSearchClock::default())
            .map_err(report)?;

        printer.out(CountOutput {
            query: self.query,
            count: answer.count,
            coverage: CoverageRow::from(&answer.coverage),
        })
    }
}

/// Show one item as the search index reads it.
///
/// Takes a hit's address, its `seq` and `--account`, and prints its
/// placements and its text by field as SEARCH.md §6 extracts it, read
/// afresh from its body, or from its summary when it holds none, with the
/// chunks its long fields split into.
#[derive(Debug, Args)]
pub struct ShowCommand {
    /// Public id of the item (`seq`), as a hit prints it.
    #[arg(value_name = "SEQ")]
    pub seq: i64,

    /// The account of the hit; omit it in a store of one account.
    #[arg(long, short = 'a', value_name = "ACCOUNT")]
    pub account: Option<String>,
}

impl ShowCommand {
    /// Prints the item.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        store.ensure_store()?;
        let index = PimdirIndex::open(store.dir()).map_err(report)?;
        let Some(item) = index
            .item(self.account.as_deref(), self.seq)
            .map_err(report)?
        else {
            anyhow::bail!(
                "No live item {} in account {}",
                self.seq,
                or_dash(self.account.as_deref())
            );
        };

        let extract = item.extract;
        let chunks = item
            .chunks
            .iter()
            .enumerate()
            .map(|(ord, chunk)| ChunkRow {
                ord,
                field: chunk.field.into(),
                start: chunk.range.start,
                end: chunk.range.end,
            })
            .collect();

        printer.out(ShowOutput {
            account: item.account,
            seq: item.seq,
            kind: item.kind,
            title: item.title,
            source: item.source.into(),
            status: extract.status.as_str().into(),
            placements: item.placements.iter().map(PlacementRow::from).collect(),
            fields: FieldsRow {
                title: extract.title,
                people: extract.people,
                body: extract.body,
                attachment: extract.attachment,
                place: extract.place,
                org: extract.org,
                note: extract.note,
                from: extract.from,
                to: extract.to,
                cc: extract.cc,
                bcc: extract.bcc,
                email: extract.email,
                organizer: extract.organizer,
                attendee: extract.attendee,
            },
            chunks,
        })
    }
}

/// Add or remove flags on every item a query matches.
///
/// Stages one `set-flags` action per placement of each hit through the
/// store's queue (pimdir SEARCH.md §10), as a producer, then applies them
/// when no sync holds the store; otherwise they wait in the queue and
/// apply at the next sync. `read`, `flagged`, `answered` and `draft` name
/// `\Seen`, `\Flagged`, `\Answered` and `\Draft`; any other flag is
/// written as given. `--dry-run` prints what would be staged and writes
/// nothing.
#[derive(Debug, Args)]
#[command(group(ArgGroup::new("change").required(true).multiple(true).args(["add", "remove"])))]
pub struct TagCommand {
    /// The query, as one argument.
    #[arg(value_name = "QUERY")]
    pub query: String,

    /// Add this flag to every hit; repeatable.
    #[arg(long, value_name = "FLAG")]
    pub add: Vec<String>,

    /// Remove this flag from every hit; repeatable.
    #[arg(long, value_name = "FLAG")]
    pub remove: Vec<String>,

    /// Print the hits, their placements and the flags each would carry,
    /// and stage nothing.
    #[arg(long)]
    pub dry_run: bool,

    #[command(flatten)]
    pub refresh: RefreshFlags,
}

impl TagCommand {
    /// Plans the tag, then stages and applies it unless dry.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let query = parse(printer, &self.query)?;
        let index = self.refresh.index(store)?;
        let plan = index
            .tag(
                &query,
                &self.add,
                &self.remove,
                store.source.as_deref(),
                &PimdirSearchClock::default(),
            )
            .map_err(report)?;

        let mut statuses = BTreeMap::new();
        let staging = plan.hits.iter().any(|hit| {
            hit.placements.iter().any(|placement| {
                placement.changes()
                    && placement.missing.is_empty()
                    && matches!(placement.source, PimdirSearchTagSource::Resolved(_))
            })
        });
        if !self.dry_run && staging {
            let actions = plan.stage(&mut store.producer()?).map_err(report)?;
            // NOTE: a drain is store-wide and applies each action as a
            // source syncing its collection, the one it runs as when that
            // one does: the source asked for, else any staged one.
            let source = store
                .source
                .clone()
                .or_else(|| actions.first().map(|action| action.source.clone()))
                .unwrap_or_default();

            // NOTE: the drain reports what it did to the whole queue, so
            // each placement itself is the only proof its action landed:
            // it carries the staged flags, or it does not.
            let owner = match store.owner_if_free()? {
                None => None,
                Some(owner) => {
                    let mut owner = owner.for_source(source);
                    match owner.drain() {
                        Err(io_pimdir::client::PimdirError::Busy) => None,
                        Err(err) => return Err(report(err)),
                        Ok(_) => Some(owner),
                    }
                }
            };
            for action in actions {
                let status = match &owner {
                    None => TagStatus::Queued,
                    Some(owner) => {
                        let item = owner
                            .get_item(&action.collection, action.seq)
                            .map_err(report)?;
                        match item {
                            Some(item) if item.flags.known() == Some(&action.flags) => {
                                TagStatus::Applied
                            }
                            _ => TagStatus::Refused,
                        }
                    }
                };
                statuses.insert(
                    (action.collection.clone(), action.seq),
                    (status, action.action),
                );
            }
        }

        let hits = plan
            .hits
            .iter()
            .map(|hit| TagHitRow {
                account: hit.account.clone(),
                seq: hit.seq,
                kind: hit.kind.clone(),
                title: hit.title.clone(),
                placements: hit
                    .placements
                    .iter()
                    .map(|placement| {
                        let staged = statuses.get(&(placement.collection.clone(), hit.seq));
                        let status = match (staged, &placement.next, &placement.source) {
                            (Some((status, _)), _, _) => *status,
                            (None, None, _) => TagStatus::UnknownFlags,
                            (None, Some(_), _) if !placement.changes() => TagStatus::Unchanged,
                            (None, Some(_), PimdirSearchTagSource::Ambiguous(_)) => {
                                TagStatus::AmbiguousSource
                            }
                            (None, Some(_), PimdirSearchTagSource::Unsynced(_)) => {
                                TagStatus::Unsynced
                            }
                            (None, Some(_), PimdirSearchTagSource::Resolved(_))
                                if !placement.missing.is_empty() =>
                            {
                                TagStatus::Unsupported
                            }
                            (None, Some(_), PimdirSearchTagSource::Resolved(_)) => {
                                TagStatus::Planned
                            }
                        };
                        let (source, sources) = match &placement.source {
                            PimdirSearchTagSource::Resolved(source) => {
                                (Some(source.clone()), vec![source.clone()])
                            }
                            PimdirSearchTagSource::Ambiguous(sources)
                            | PimdirSearchTagSource::Unsynced(sources) => (None, sources.clone()),
                        };
                        TagPlacementRow {
                            collection: placement.collection.clone(),
                            link_id: placement.link_id.0.clone(),
                            flags: placement
                                .flags
                                .known()
                                .map(|flags| flags.iter().cloned().collect()),
                            next: placement
                                .next
                                .as_ref()
                                .map(|flags| flags.iter().cloned().collect()),
                            status,
                            source,
                            sources,
                            syncs: placement.syncs,
                            missing: placement.missing.clone(),
                            action: staged.map(|(_, action)| *action),
                        }
                    })
                    .collect(),
            })
            .collect();

        printer.out(TagOutput {
            query: self.query,
            dry_run: self.dry_run,
            add: plan.add.into_iter().collect(),
            remove: plan.remove.into_iter().collect(),
            hits,
            coverage: CoverageRow::from(&plan.coverage),
        })
    }
}

/// Show the mail thread of one item.
///
/// Takes a hit's address, its `seq` and `--account`, and prints its
/// thread (pimdir SEARCH.md §9): the thread id `thread:` takes, how many
/// messages it holds in the account, its first and last date, and its
/// members there, oldest first. An item other than mail, or a message the
/// index has not threaded, is its own thread.
#[derive(Debug, Args)]
pub struct ThreadCommand {
    /// Public id of the item (`seq`), as a hit prints it.
    #[arg(value_name = "SEQ")]
    pub seq: i64,

    /// The account of the hit; omit it in a store of one account.
    #[arg(long, short = 'a', value_name = "ACCOUNT")]
    pub account: Option<String>,

    #[command(flatten)]
    pub refresh: RefreshFlags,
}

impl ThreadCommand {
    /// Prints the thread.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let index = self.refresh.index(store)?;
        let Some(thread) = index
            .thread(self.account.as_deref(), self.seq)
            .map_err(report)?
        else {
            anyhow::bail!(
                "No live item {} in account {}",
                self.seq,
                or_dash(self.account.as_deref())
            );
        };

        printer.out(ThreadRow::from(&thread))
    }
}

/// How long a one-shot answer may spend folding the store's changes in.
#[derive(Debug, Args)]
pub struct RefreshFlags {
    /// Stop folding the store's changes into the index after this long
    /// (`2s`, `500ms`, `0s` to answer from the index as it stands).
    ///
    /// What is left unfolded is counted in the coverage's `unindexed`; run
    /// `index` to fold everything.
    #[arg(long, value_name = "DURATION", default_value = "2s")]
    pub refresh_timeout: humantime::Duration,
}

impl RefreshFlags {
    /// Opens the index and refreshes it until the deadline.
    fn index(&self, store: &StoreFlags) -> Result<PimdirIndex> {
        store.ensure_store()?;
        let mut index = PimdirIndex::open(store.dir()).map_err(report)?;
        let timeout: Duration = self.refresh_timeout.into();
        if !timeout.is_zero() {
            index
                .refresh(&PimdirIndexRefreshOptions {
                    deadline: Some(Instant::now() + timeout),
                    ..Default::default()
                })
                .map_err(report)?;
        }
        Ok(index)
    }
}

/// The orders a search takes.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum SortArg {
    /// Newest first, undated items (contacts, files) last.
    Date,
    /// By title.
    Name,
    /// Best text match first.
    Relevance,
}

impl From<SortArg> for PimdirSearchSort {
    fn from(sort: SortArg) -> Self {
        match sort {
            SortArg::Date => Self::Date,
            SortArg::Name => Self::Name,
            SortArg::Relevance => Self::Relevance,
        }
    }
}

/// Parses a query, printing its refusal and exiting when refused.
fn parse(printer: &mut impl Printer, query: &str) -> Result<PimdirQuery> {
    match PimdirQuery::parse(query) {
        Ok(query) => Ok(query),
        Err(err) => refuse(printer, query, err),
    }
}

/// Prints a refusal (SEARCH §8.4) and exits with status 1.
fn refuse<T>(printer: &mut impl Printer, query: &str, err: PimdirQueryError) -> Result<T> {
    printer.out(QueryErrorOutput {
        query: query.into(),
        code: err.code.as_str().into(),
        message: err.message,
        start: err.start,
        end: err.end,
        expected: err.expected,
        fields: err.fields,
    })?;
    process::exit(1)
}

fn level_name(level: PimdirLevel) -> &'static str {
    match level {
        PimdirLevel::Meta => "meta",
        PimdirLevel::Full => "full",
    }
}

/// The `index` output.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IndexOutput {
    /// Change feed rows folded in.
    pub folded: u64,
    /// Bodies extracted and indexed for the first time.
    pub indexed: u64,
    /// Placements dropped: deleted, retained, purged or renamed away.
    pub removed: u64,
    /// Whether the keys were reconciled against the store.
    pub reconciled: bool,
    /// The store's last change stamp when the refresh began.
    pub head: i64,
    /// The last change stamp the index has folded in.
    pub cursor: i64,
    /// Whether another indexer was running, so this one did nothing.
    pub busy: bool,
    /// Whether the index holds every change the store had.
    pub current: bool,
}

impl fmt::Display for IndexOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.busy {
            return writeln!(
                f,
                "Another indexer is running: the index stands at change {} of {}",
                self.cursor, self.head
            );
        }
        writeln!(
            f,
            "Folded {} change(s), indexed {} body(ies), dropped {} placement(s): the index stands at change {} of {}",
            self.folded, self.indexed, self.removed, self.cursor, self.head
        )
    }
}

/// The `search` output: one page of hits and what they could not see.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchOutput {
    /// The query as given.
    pub query: String,
    /// The hits, in the order asked.
    pub hits: Vec<HitRow>,
    /// The `--offset` of the next page, `null` on the last.
    pub next_offset: Option<usize>,
    /// What the answer could not see.
    pub coverage: CoverageRow,
}

impl fmt::Display for SearchOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.hits.is_empty() {
            writeln!(f, "No hit")?;
        }
        for hit in &self.hits {
            let account = hit
                .account
                .as_deref()
                .map(|account| format!(" --account {account}"))
                .unwrap_or_default();
            writeln!(f, "{}{account} [{}] {}", hit.seq, hit.kind, hit.title)?;
            let collections: Vec<&str> = hit
                .placements
                .iter()
                .map(|placement| placement.collection.as_str())
                .collect();
            writeln!(
                f,
                " - in {}, matched {}",
                collections.join(", "),
                hit.matched.map(Matched::as_str).unwrap_or("by its fields")
            )?;
            if let Some(snippet) = &hit.snippet {
                writeln!(f, " - {snippet}")?;
            }
        }
        if let Some(offset) = self.next_offset {
            writeln!(f, "More hits follow: pass --offset {offset}")?;
        }
        write!(f, "{}", self.coverage)
    }
}

/// The `search --threads` output: one page of threads and what their hits
/// could not see.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchThreadsOutput {
    /// The query as given.
    pub query: String,
    /// The threads, in the order of their first hit.
    pub threads: Vec<ThreadRow>,
    /// The `--offset` of the next page, `null` on the last.
    pub next_offset: Option<usize>,
    /// What the answer could not see.
    pub coverage: CoverageRow,
}

impl fmt::Display for SearchThreadsOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.threads.is_empty() {
            writeln!(f, "No hit")?;
        }
        for thread in &self.threads {
            let account = thread
                .account
                .as_deref()
                .map(|account| format!(" in account {account}"))
                .unwrap_or_default();
            match &thread.thread {
                Some(id) => writeln!(
                    f,
                    "Thread {id}{account}: {} message(s), {} to {}",
                    thread.count,
                    or_dash(thread.first.as_deref()),
                    or_dash(thread.last.as_deref())
                )?,
                None => writeln!(f, "No thread{account}")?,
            }
            for hit in &thread.hits {
                writeln!(f, " - {} [{}] {}", hit.seq, hit.kind, hit.title)?;
            }
        }
        if let Some(offset) = self.next_offset {
            writeln!(f, "More threads follow: pass --offset {offset}")?;
        }
        write!(f, "{}", self.coverage)
    }
}

/// The hits of one thread in one account, or a hit no thread holds.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRow {
    /// The account, `null` in a store of one account.
    pub account: Option<String>,
    /// The thread id, which `thread:` takes; `null` for an item other
    /// than mail, which no thread holds.
    pub thread: Option<String>,
    /// The thread's live messages in the account, hit or not; 1 outside
    /// a thread.
    pub count: u64,
    /// The earliest date among them, `null` without one or outside a
    /// thread.
    pub first: Option<String>,
    /// The latest.
    pub last: Option<String>,
    /// Its hits, in the order asked.
    pub hits: Vec<HitRow>,
}

impl From<&PimdirSearchThread> for ThreadRow {
    fn from(thread: &PimdirSearchThread) -> Self {
        Self {
            account: thread.account.clone(),
            thread: thread.thread.clone(),
            count: thread.count,
            first: thread.first.clone(),
            last: thread.last.clone(),
            hits: thread.hits.iter().map(HitRow::from).collect(),
        }
    }
}

impl fmt::Display for ThreadRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let account = self
            .account
            .as_deref()
            .map(|account| format!(" in account {account}"))
            .unwrap_or_default();
        match &self.thread {
            Some(id) => writeln!(
                f,
                "Thread {id}{account}: {} message(s), {} to {}",
                self.count,
                or_dash(self.first.as_deref()),
                or_dash(self.last.as_deref())
            )?,
            None => writeln!(f, "No thread{account}: the item stands alone")?,
        }
        for hit in &self.hits {
            writeln!(f, " - {} {} {}", hit.seq, hit.sort_key, hit.title)?;
        }
        Ok(())
    }
}

/// The `tag` output: every hit of the query, each placement with the
/// flags it carries, the flags the tag gives it and what became of that.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TagOutput {
    /// The query as given.
    pub query: String,
    /// Whether this was a `--dry-run`, which staged nothing.
    pub dry_run: bool,
    /// The flags added, `is:` spellings mapped to their flags.
    pub add: Vec<String>,
    /// The flags removed.
    pub remove: Vec<String>,
    /// Every hit, newest first.
    pub hits: Vec<TagHitRow>,
    /// What the query could not see: a tag reaches only the hits the
    /// index answers.
    pub coverage: CoverageRow,
}

impl fmt::Display for TagOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.hits.is_empty() {
            writeln!(f, "No hit: nothing to tag")?;
        }
        let mut statuses = BTreeMap::new();
        for hit in &self.hits {
            writeln!(f, "{} [{}] {}", hit.seq, hit.kind, hit.title)?;
            for placement in &hit.placements {
                *statuses.entry(placement.status.as_str()).or_insert(0) += 1;
                let next = placement
                    .next
                    .as_ref()
                    .map(|flags| flags.join(" "))
                    .unwrap_or_else(|| "-".into());
                writeln!(
                    f,
                    " - in {}: {} ({})",
                    placement.collection,
                    next,
                    placement.status.as_str()
                )?;
                if !placement.missing.is_empty() {
                    writeln!(
                        f,
                        "   not staged: its source supports no {}",
                        placement.missing.join(", ")
                    )?;
                } else if !placement.syncs && placement.source.is_some() {
                    let why = match hit.kind.as_str() {
                        "mail" => "its source declares no support for these flags",
                        _ => "its source carries no flags for this kind",
                    };
                    writeln!(f, "   kept in the store only: {why}")?;
                }
            }
        }
        for (status, count) in &statuses {
            writeln!(f, "{count} placement(s) {status}")?;
        }
        if statuses.contains_key(TagStatus::Queued.as_str()) {
            writeln!(
                f,
                "Another writer holds the store lock: the queued actions apply at the next sync"
            )?;
        }
        if statuses.contains_key(TagStatus::AmbiguousSource.as_str()) {
            writeln!(
                f,
                "Several sources sync a collection: pick the one to write as with --source"
            )?;
        }
        if statuses.contains_key(TagStatus::Refused.as_str()) {
            writeln!(
                f,
                "Some actions were not applied: see `pimdir queue list --parked`"
            )?;
        }
        write!(f, "{}", self.coverage)
    }
}

/// One hit a tag reaches.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TagHitRow {
    /// The account, `null` in a store of one account.
    pub account: Option<String>,
    /// The public id.
    pub seq: i64,
    /// The kind, as a search hit prints it.
    pub kind: String,
    /// Its subject, name or summary.
    pub title: String,
    /// Its live placements in the account.
    pub placements: Vec<TagPlacementRow>,
}

/// One placement a tag reaches.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TagPlacementRow {
    /// The collection.
    pub collection: String,
    /// The item's key there.
    pub link_id: String,
    /// Its flags before the tag, `null` when never read.
    pub flags: Option<Vec<String>>,
    /// The flags the tag gives it, `null` when its flags were never read.
    pub next: Option<Vec<String>>,
    /// What became of it.
    pub status: TagStatus,
    /// The source its action is staged as: the one asked for with
    /// `--source`, else the only one syncing the collection; `null` when
    /// none is.
    pub source: Option<String>,
    /// The sources syncing the collection: the one staging, or the
    /// candidates of an ambiguous one, empty when none syncs it.
    pub sources: Vec<String>,
    /// Whether its source carries the tag to its server by the
    /// capabilities it declares: only mail has flags there. False keeps
    /// the tag in the store alone, whatever its status.
    pub syncs: bool,
    /// The capabilities the change needs that its source declares no
    /// support for; not empty, the placement is `unsupported` and nothing
    /// is staged for it.
    pub missing: Vec<String>,
    /// The queue row staged for it, `null` when none was.
    pub action: Option<i64>,
}

/// What became of one placement a tag reaches.
#[derive(Clone, Copy, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum TagStatus {
    /// A dry run: an action would be staged.
    Planned,
    /// It already carries the flags: nothing staged.
    Unchanged,
    /// Its flags were never read, so no flag set can be staged without
    /// losing them: nothing staged.
    UnknownFlags,
    /// Several sources sync its collection and `--source` picked none of
    /// them: nothing staged, `sources` lists them.
    AmbiguousSource,
    /// The source asked for does not sync its collection, or none does:
    /// nothing staged.
    Unsynced,
    /// Its source declares no support for a capability the change needs
    /// (`missing`): nothing staged.
    Unsupported,
    /// Staged and applied: the store shows the flags.
    Applied,
    /// Staged while another writer holds the store lock, or while the store
    /// is of another revision: applies at the next sync.
    Queued,
    /// Staged, drained, and the store does not show the flags: the action
    /// was refused, see `pimdir queue list --parked`.
    Refused,
}

impl TagStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Unchanged => "unchanged",
            Self::UnknownFlags => "unknownFlags",
            Self::AmbiguousSource => "ambiguousSource",
            Self::Unsynced => "unsynced",
            Self::Unsupported => "unsupported",
            Self::Applied => "applied",
            Self::Queued => "queued",
            Self::Refused => "refused",
        }
    }
}

/// The `count` output.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CountOutput {
    /// The query as given.
    pub query: String,
    /// How many hits it has.
    pub count: u64,
    /// What the count could not see.
    pub coverage: CoverageRow,
}

impl fmt::Display for CountOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{} hit(s)", self.count)?;
        write!(f, "{}", self.coverage)
    }
}

/// One hit: an item in one account.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HitRow {
    /// The account, `null` in a store of one account; with `seq`, the
    /// address `show` takes.
    pub account: Option<String>,
    /// The public id.
    pub seq: i64,
    /// `mail`, `contact`, `event`, `task`, `journal` or `file`, else the
    /// collection's media type.
    pub kind: String,
    /// Its subject, name or summary.
    pub title: String,
    /// Its sort key.
    pub sort_key: String,
    /// What a text term matched, `null` when only field terms did.
    pub matched: Option<Matched>,
    /// A passage of its body, `null` without one.
    pub snippet: Option<String>,
    /// Its live placements in the account.
    pub placements: Vec<PlacementRow>,
}

impl From<&PimdirSearchHit> for HitRow {
    fn from(hit: &PimdirSearchHit) -> Self {
        Self {
            account: hit.account.clone(),
            seq: hit.seq,
            kind: hit.kind.clone(),
            title: hit.title.clone(),
            sort_key: hit.sort_key.clone(),
            matched: hit.matched.map(Matched::from),
            snippet: hit.snippet.clone(),
            placements: hit.placements.iter().map(PlacementRow::from).collect(),
        }
    }
}

/// One placement of a hit.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlacementRow {
    /// The collection.
    pub collection: String,
    /// The item's key there.
    pub link_id: String,
    /// Its flags there, `null` when unknown.
    pub flags: Option<Vec<String>>,
    /// The body's hash, `null` while bodiless.
    pub object: Option<String>,
    /// `meta` or `full`.
    pub level: String,
    /// Its sort key.
    pub sort_key: String,
}

impl From<&PimdirSearchPlacement> for PlacementRow {
    fn from(placement: &PimdirSearchPlacement) -> Self {
        Self {
            collection: placement.collection.clone(),
            link_id: placement.link_id.0.clone(),
            flags: placement
                .flags
                .known()
                .map(|flags| flags.iter().cloned().collect()),
            object: placement.object.as_ref().map(|hash| hash.0.clone()),
            level: level_name(placement.level).into(),
            sort_key: placement.sort_key.clone(),
        }
    }
}

/// What a text term matched, or what an item's text was read from.
#[derive(Clone, Copy, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Matched {
    /// The body.
    Body,
    /// The summary of an item holding no body.
    Summary,
}

impl Matched {
    fn as_str(self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::Summary => "summary",
        }
    }
}

impl From<PimdirSearchMatched> for Matched {
    fn from(matched: PimdirSearchMatched) -> Self {
        match matched {
            PimdirSearchMatched::Body => Self::Body,
            PimdirSearchMatched::Summary => Self::Summary,
        }
    }
}

/// What an answer could not see, on three axes.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CoverageRow {
    /// Whether nothing was out of sight: the answer is the whole answer.
    pub complete: bool,
    /// Live items in scope whose content is not searchable here.
    pub local: LocalRow,
    /// The range the store is known to hold every member of.
    pub temporal: TemporalRow,
    /// Whether a server-side search was merged in; always `localOnly`
    /// here.
    pub remote: RemoteRow,
    /// Whether the date range leaves the horizon calendar occurrences are
    /// materialised within, a calendar in scope.
    pub horizon_left: bool,
}

impl From<&PimdirSearchCoverage> for CoverageRow {
    fn from(coverage: &PimdirSearchCoverage) -> Self {
        Self {
            complete: coverage.is_complete(),
            local: LocalRow {
                bodiless: coverage.local.bodiless.clone(),
                unindexed: coverage.local.unindexed,
                behind: coverage.local.behind,
            },
            temporal: match &coverage.temporal {
                PimdirSearchTemporal::Full => TemporalRow::Full,
                PimdirSearchTemporal::Bounded {
                    since,
                    until,
                    filling,
                } => TemporalRow::Bounded {
                    since: since.clone(),
                    until: until.clone(),
                    filling: *filling,
                },
            },
            remote: match coverage.remote {
                PimdirSearchRemote::LocalOnly => RemoteRow::LocalOnly,
                PimdirSearchRemote::Augmented { exhaustive } => RemoteRow::Augmented { exhaustive },
            },
            horizon_left: coverage.horizon_left,
        }
    }
}

impl fmt::Display for CoverageRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.complete {
            return writeln!(f, "Coverage: complete");
        }
        writeln!(f, "Coverage: partial")?;
        for (kind, count) in &self.local.bodiless {
            if *count > 0 {
                writeln!(
                    f,
                    " - {count} {kind} item(s) without a body, searched by summary"
                )?;
            }
        }
        if self.local.unindexed > 0 {
            writeln!(
                f,
                " - {} item(s) changed since the index was refreshed",
                self.local.unindexed
            )?;
        }
        if self.local.behind > 0 {
            writeln!(
                f,
                " - the index is {} change(s) behind the store",
                self.local.behind
            )?;
        }
        if let TemporalRow::Bounded {
            since,
            until,
            filling,
        } = &self.temporal
        {
            if let Some(since) = since {
                writeln!(f, " - items before {since} are not all held")?;
            }
            if let Some(until) = until {
                writeln!(f, " - items from {until} on are not all held")?;
            }
            if *filling {
                writeln!(f, " - a first sync is still filling a collection in")?;
            }
        }
        if let RemoteRow::Augmented { exhaustive: false } = self.remote {
            writeln!(f, " - the server answered in part")?;
        }
        if self.horizon_left {
            writeln!(
                f,
                " - the range leaves the horizon calendar occurrences are indexed within"
            )?;
        }
        Ok(())
    }
}

/// The local axis.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LocalRow {
    /// Live items holding no body, per collection media type.
    pub bodiless: BTreeMap<String, u64>,
    /// Live items whose last change the index has not folded in yet.
    pub unindexed: u64,
    /// The store's changes the index has not folded in, store-wide.
    pub behind: u64,
}

/// The temporal axis.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum TemporalRow {
    /// Every queried collection holds every member of the range.
    Full,
    /// Some queried collection is not known to.
    #[serde(rename_all = "camelCase")]
    Bounded {
        /// The instant below which members are not all held, if that is
        /// a cause.
        since: Option<String>,
        /// The instant from which they are not all held, if that is a
        /// cause.
        until: Option<String>,
        /// Whether a source never closed a round over a queried
        /// collection.
        filling: bool,
    },
}

/// The remote axis.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum RemoteRow {
    /// Only the local store was searched.
    LocalOnly,
    /// A server-side search was merged in.
    Augmented {
        /// Whether the server answered every match.
        exhaustive: bool,
    },
}

/// The refusal of a query (SEARCH §8.4), printed by `search` and `count`
/// before they exit with status 1.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QueryErrorOutput {
    /// The query as given.
    pub query: String,
    /// `empty-query`, `unknown-field`, `invalid-value`, `invalid-date`,
    /// `unterminated-phrase`, `unbalanced-parenthesis`,
    /// `dangling-operator` or `invalid-wildcard`.
    pub code: String,
    /// The fault, as a sentence.
    pub message: String,
    /// The first character at fault, counted in Unicode scalar values.
    pub start: usize,
    /// The character after the last at fault.
    pub end: usize,
    /// What would have been valid at `start`.
    pub expected: Vec<String>,
    /// Every field name, for a field refused.
    pub fields: Vec<String>,
}

impl fmt::Display for QueryErrorOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Query refused ({}): {}", self.code, self.message)?;
        writeln!(f, "  {}", self.query)?;
        writeln!(
            f,
            "  {}{}",
            " ".repeat(self.start),
            "^".repeat((self.end - self.start).max(1))
        )?;
        if !self.expected.is_empty() {
            writeln!(f, "Expected: {}", self.expected.join(", "))?;
        }
        if !self.fields.is_empty() {
            writeln!(f, "Fields: {}", self.fields.join(", "))?;
        }
        Ok(())
    }
}

/// The `show` output: an item as the index reads it.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ShowOutput {
    /// The account, `null` in a store of one account.
    pub account: Option<String>,
    /// The public id.
    pub seq: i64,
    /// The kind, as a hit prints it.
    pub kind: String,
    /// Its subject, name or summary.
    pub title: String,
    /// What the text was read from: `body`, or `summary` for an item
    /// holding none.
    pub source: Matched,
    /// `ok`, `encrypted` or `unparseable`.
    pub status: String,
    /// Its live placements in the account.
    pub placements: Vec<PlacementRow>,
    /// Its text by field.
    pub fields: FieldsRow,
    /// The chunks its `body` and `note` fields split into.
    pub chunks: Vec<ChunkRow>,
}

impl fmt::Display for ShowOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let account = self
            .account
            .as_deref()
            .map(|account| format!(" in account {account}"))
            .unwrap_or_default();
        writeln!(
            f,
            "Item {}{account} [{}] {}",
            self.seq, self.kind, self.title
        )?;
        for placement in &self.placements {
            writeln!(
                f,
                " - in {} as {} ({})",
                placement.collection, placement.link_id, placement.level
            )?;
        }
        writeln!(
            f,
            " - read from its {}, {}",
            self.source.as_str(),
            self.status
        )?;

        let fields = [
            ("people", &self.fields.people),
            ("attachments", &self.fields.attachment),
            ("place", &self.fields.place),
            ("org", &self.fields.org),
        ];
        for (name, text) in fields {
            if !text.is_empty() {
                writeln!(f, " - {name}: {}", text.replace('\n', "; "))?;
            }
        }
        for text in [&self.fields.body, &self.fields.note] {
            if !text.is_empty() {
                writeln!(f)?;
                writeln!(f, "{}", text.trim_end())?;
            }
        }
        Ok(())
    }
}

/// An item's text by field (SEARCH §6).
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FieldsRow {
    /// A subject, a name or a summary.
    pub title: String,
    /// Every name and address the item names.
    pub people: String,
    /// The text parts of a message.
    pub body: String,
    /// Its attachments' names and media types.
    pub attachment: String,
    /// A location.
    pub place: String,
    /// An organisation, a title or a role.
    pub org: String,
    /// Notes and telephone numbers.
    pub note: String,
    /// The `from` role's names and addresses.
    pub from: String,
    /// The `to` role's.
    pub to: String,
    /// The `cc` role's.
    pub cc: String,
    /// The `bcc` role's.
    pub bcc: String,
    /// The `email` role's.
    pub email: String,
    /// The `organizer` role's.
    pub organizer: String,
    /// The `attendee` role's.
    pub attendee: String,
}

/// One chunk of a long field.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChunkRow {
    /// Its rank, from 0, across `body` then `note`.
    pub ord: usize,
    /// `body` or `note`.
    pub field: String,
    /// Its first UTF-8 byte in the field's text.
    pub start: usize,
    /// The byte after its last.
    pub end: usize,
}
