//! The search verbs over the store's index (pimdir SEARCH.md): `index`,
//! `search`, `count`, `show`, `thread` and `tag`, the one that writes,
//! through the queue.
//!
//! They read content as SEARCH §6 extracts it, which is the spec's rule
//! and not this tool's opinion, and never render it: a hit prints as its
//! address, `(account, seq)`, its placements, what it matched and a
//! passage of its text. `search` and `count` refresh the index first,
//! within `--refresh-timeout`, and say in their coverage what they could
//! not fold in. A refused query (SEARCH §8.4) fails with `query-refused`,
//! its message saying where it is wrong and what would be accepted.

use std::{
    collections::BTreeMap,
    fmt,
    time::{Duration, Instant},
};

use anyhow::Result;
use clap::{ArgGroup, Args, ValueEnum};
use io_pimdir::{
    client::search::{
        PimdirIndex, PimdirIndexRefreshOptions, PimdirSearchClock, PimdirSearchHit,
        PimdirSearchMatched, PimdirSearchOptions, PimdirSearchPlacement, PimdirSearchSort,
        PimdirSearchThread,
    },
    placement::PimdirLevel,
    search::{
        coverage::{
            PimdirSearchCoverage, PimdirSearchReads, PimdirSearchRemote, PimdirSearchTemporal,
        },
        query::PimdirQuery,
        tag::PimdirSearchTagSource,
    },
};
use pimalaya_cli::{
    columns::Columns,
    fields::Fields,
    printer::Printer,
    records::{Record, Records},
    sanitize,
};
use schemars::JsonSchema;
use serde::Serialize;

use crate::cli::{
    StoreFlags,
    error::{ErrorCode, partial, refusal, report},
    or_dash,
};

/// The default page size of a search.
const DEFAULT_LIMIT: usize = 50;

/// Fold the store's changes into its search index.
///
/// Creates index.db beside the store when absent, rebuilds it when it was
/// built by another version, and folds the whole change feed in. Reads the
/// store as a reader, so it runs beside a sync; another indexer running
/// makes it report so and do nothing.
#[derive(Debug, Args)]
pub struct IndexCommand {
    /// The IANA zone floating times and dates are read in (`UTC`,
    /// `Europe/Paris`).
    ///
    /// The index records it: another zone re-expands every calendar item,
    /// so pass the same one to every command refreshing the index.
    #[arg(long, value_name = "ZONE", default_value = "UTC")]
    pub floating_zone: String,
}

impl IndexCommand {
    /// Runs a full refresh.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        store.ensure_store()?;
        let mut index = PimdirIndex::open(store.dir()).map_err(report)?;
        let report = index
            .refresh(&PimdirIndexRefreshOptions {
                floating_zone: self.floating_zone,
                ..Default::default()
            })
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

    /// Resume a listing at the given page.
    ///
    /// The token is the `nextPage` value a previous listing printed, given
    /// back as it is.
    #[arg(long = "page-token", alias = "offset", value_name = "TOKEN")]
    pub page_token: Option<String>,

    /// Collapse the hits to their mail threads.
    ///
    /// Prints one entry per thread in each account, with its id (which
    /// `thread:` takes), how many messages it holds there, its first and
    /// last date and its hits; an item other than mail stands alone.
    /// `--limit` and `--page-token` then count threads.
    #[arg(long)]
    pub threads: bool,

    /// The IANA zone the query's days are read in (`UTC`, `Europe/Paris`),
    /// the index's floating zone by default.
    #[arg(long, value_name = "ZONE")]
    pub zone: Option<String>,

    #[command(flatten)]
    pub refresh: RefreshFlags,
}

impl SearchCommand {
    /// Prints one page of hits with the coverage.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let query = parse(&self.query)?;
        let index = self.refresh.index(store)?;
        let limit = self.limit.max(1);
        // NOTE: the token is the offset of the next page, as its decimal.
        let offset = match &self.page_token {
            None => 0,
            Some(token) => token.parse::<usize>().map_err(|_| {
                ErrorCode::PageTokenInvalid.error(format!(
                    "--page-token takes the nextPage a listing printed, got {token:?}"
                ))
            })?,
        };
        let options = PimdirSearchOptions {
            sort: self.sort.into(),
            limit,
            offset,
            clock: clock(&index, self.zone)?,
        };

        if self.threads {
            let answer = index.threads(&query, &options).map_err(report)?;

            return printer.out(SearchThreadsOutput {
                query: self.query,
                next_page: answer
                    .more
                    .then(|| (offset + answer.threads.len()).to_string()),
                threads: answer.threads.iter().map(ThreadRow::from).collect(),
                coverage: CoverageRow::from(&answer.coverage),
            });
        }

        let answer = index.search(&query, &options).map_err(report)?;

        printer.out(SearchOutput {
            query: self.query,
            next_page: answer
                .more
                .then(|| (offset + answer.hits.len()).to_string()),
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

    /// The IANA zone the query's days are read in (`UTC`, `Europe/Paris`),
    /// the index's floating zone by default.
    #[arg(long, value_name = "ZONE")]
    pub zone: Option<String>,

    #[command(flatten)]
    pub refresh: RefreshFlags,
}

impl CountCommand {
    /// Prints the count with the coverage.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let query = parse(&self.query)?;
        let index = self.refresh.index(store)?;

        let answer = index
            .count(&query, &clock(&index, self.zone)?)
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
            return Err(ErrorCode::ItemNotFound.error(format!(
                "no live item {} in account {}",
                self.seq,
                or_dash(self.account.as_deref())
            )));
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
///
/// Exits 2 once the result is printed when a placement is not tagged and a
/// run would not tag it: unknown flags, an ambiguous or unsynced source, an
/// unsupported change, or an action the store refused.
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

    /// The IANA zone the query's days are read in (`UTC`, `Europe/Paris`),
    /// the index's floating zone by default.
    #[arg(long, value_name = "ZONE")]
    pub zone: Option<String>,

    #[command(flatten)]
    pub refresh: RefreshFlags,
}

impl TagCommand {
    /// Plans the tag, then stages and applies it unless dry.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let query = parse(&self.query)?;
        let index = self.refresh.index(store)?;
        let plan = index
            .tag(
                &query,
                &self.add,
                &self.remove,
                store.source.as_deref(),
                &clock(&index, self.zone)?,
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

        let hits: Vec<TagHitRow> = plan
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
                            row: staged.map(|(_, row)| row.id),
                            handle: staged.map(|(_, row)| row.to_string()),
                        }
                    })
                    .collect(),
            })
            .collect();

        let short = hits
            .iter()
            .flat_map(|hit| &hit.placements)
            .any(|placement| placement.status.falls_short());

        printer.out(TagOutput {
            query: self.query,
            dry_run: self.dry_run,
            add: plan.add.into_iter().collect(),
            remove: plan.remove.into_iter().collect(),
            hits,
            coverage: CoverageRow::from(&plan.coverage),
        })?;

        match short {
            true => Err(partial()),
            false => Ok(()),
        }
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
            return Err(ErrorCode::ItemNotFound.error(format!(
                "no live item {} in account {}",
                self.seq,
                or_dash(self.account.as_deref())
            )));
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

    /// The IANA zone floating times and dates are read in (`UTC`,
    /// `Europe/Paris`), as `index --floating-zone` states it.
    ///
    /// The index records it: another zone re-expands every calendar item.
    #[arg(long, value_name = "ZONE", default_value = "UTC")]
    pub floating_zone: String,
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
                    floating_zone: self.floating_zone.clone(),
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

/// Parses a query, failing with its refusal (SEARCH §8.4).
fn parse(query: &str) -> Result<PimdirQuery> {
    PimdirQuery::parse(query).map_err(|err| refusal(query, err))
}

/// A query's clock in `zone`, else in the index's floating zone.
fn clock(index: &PimdirIndex, zone: Option<String>) -> Result<PimdirSearchClock> {
    let zone = match zone {
        Some(zone) => zone,
        None => index.floating_zone().map_err(report)?,
    };
    Ok(PimdirSearchClock {
        now: None,
        zone: Some(zone),
    })
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
    /// The token `--page-token` resumes from, `null` on the last page.
    pub next_page: Option<String>,
    /// What the answer could not see.
    pub coverage: CoverageRow,
}

impl fmt::Display for SearchOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.hits.is_empty() {
            writeln!(f, "No hit")?;
        } else {
            let records = Records {
                items: self.hits.iter().map(HitRow::record).collect(),
            };
            writeln!(f, "{records}")?;
        }
        if let Some(token) = &self.next_page {
            writeln!(f, "More hits follow: pass --page-token {token}")?;
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
    /// The token `--page-token` resumes from, `null` on the last page.
    pub next_page: Option<String>,
    /// What the answer could not see.
    pub coverage: CoverageRow,
}

impl fmt::Display for SearchThreadsOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.threads.is_empty() {
            writeln!(f, "No hit")?;
        } else {
            let mut records = Records::default();

            for thread in &self.threads {
                let mut fields = Fields::default();
                let mut hits = Columns::default();

                if let Some(account) = &thread.account {
                    fields.push("Account", sanitize::text(account));
                }

                let first = or_dash(thread.first.as_deref());
                let last = or_dash(thread.last.as_deref());
                fields.push("Messages", thread.count);
                fields.push("First", sanitize::text(first));
                fields.push("Last", sanitize::text(last));

                for hit in &thread.hits {
                    hits.push([
                        hit.seq.to_string(),
                        sanitize::text(&hit.kind).into_owned(),
                        sanitize::text(&hit.title).into_owned(),
                    ]);
                }

                records.push(Record {
                    title: sanitize::text(or_dash(thread.thread.as_deref())).into_owned(),
                    state: Vec::new(),
                    fields,
                    body: hits.to_string(),
                });
            }

            writeln!(f, "{records}")?;
        }
        if let Some(token) = &self.next_page {
            writeln!(f, "More threads follow: pass --page-token {token}")?;
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
        let mut fields = Fields::default();

        fields.push("Thread", sanitize::text(or_dash(self.thread.as_deref())));

        if let Some(account) = &self.account {
            fields.push("Account", sanitize::text(account));
        }

        fields.push("Messages", self.count);
        fields.push("First", sanitize::text(or_dash(self.first.as_deref())));
        fields.push("Last", sanitize::text(or_dash(self.last.as_deref())));

        write!(f, "{fields}")?;

        if self.hits.is_empty() {
            return Ok(());
        }

        let records = Records {
            items: self.hits.iter().map(HitRow::record).collect(),
        };

        write!(f, "\n{records}")
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
        let mut statuses = BTreeMap::new();
        let mut records = Records::default();

        for hit in &self.hits {
            let mut fields = Fields::default();

            if let Some(account) = &hit.account {
                fields.push("Account", sanitize::text(account));
            }

            fields.push("Kind", sanitize::text(&hit.kind));
            fields.push("Title", sanitize::text(&hit.title));

            for placement in &hit.placements {
                let status = placement.status.as_str();
                let next = placement.next.as_ref().map(|flags| flags.join(" "));
                let mut value = format!(
                    "{}: {} ({status})",
                    sanitize::text(&placement.collection),
                    sanitize::text(next.as_deref().unwrap_or("-")),
                );

                if !placement.missing.is_empty() {
                    let missing = placement.missing.join(", ");
                    let missing = sanitize::text(&missing);
                    value.push_str(&format!("\nnot staged: its source supports no {missing}"));
                } else if !placement.syncs && placement.source.is_some() {
                    let why = match hit.kind.as_str() {
                        "mail" => "its source declares no support for these flags",
                        _ => "its source carries no flags for this kind",
                    };
                    value.push_str(&format!("\nkept in the store only: {why}"));
                }

                *statuses.entry(status).or_insert(0) += 1;
                fields.push("Placement", value);
            }

            records.push(Record {
                title: hit.seq.to_string(),
                state: Vec::new(),
                fields,
                body: String::new(),
            });
        }

        if self.hits.is_empty() {
            writeln!(f, "No hit: nothing to tag")?;
        } else {
            writeln!(f, "{records}")?;
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
    pub row: Option<i64>,
    /// The row's handle, its id under the store's id and incarnation,
    /// `null` when none was staged.
    pub handle: Option<String>,
}

/// What became of one placement a tag reaches.
#[derive(Clone, Copy, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
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
            Self::UnknownFlags => "unknown-flags",
            Self::AmbiguousSource => "ambiguous-source",
            Self::Unsynced => "unsynced",
            Self::Unsupported => "unsupported",
            Self::Applied => "applied",
            Self::Queued => "queued",
            Self::Refused => "refused",
        }
    }

    /// Whether the placement does not end up tagged, nor will by a run
    /// without `--dry-run`: the tag exits 2.
    fn falls_short(self) -> bool {
        match self {
            Self::Planned | Self::Unchanged | Self::Applied | Self::Queued => false,
            Self::UnknownFlags
            | Self::AmbiguousSource
            | Self::Unsynced
            | Self::Unsupported
            | Self::Refused => true,
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

impl HitRow {
    /// The hit as a text list prints it, its snippet as the body.
    fn record(&self) -> Record<'static> {
        let placements = self.placements.iter();
        let collections = placements.map(|placement| placement.collection.as_str());
        let collections = collections.collect::<Vec<_>>().join(", ");
        let sort_key = Some(self.sort_key.as_str()).filter(|key| !key.is_empty());
        let mut fields = Fields::default();

        if let Some(account) = &self.account {
            fields.push("Account", sanitize::text(account));
        }

        fields.push("Kind", sanitize::text(&self.kind));
        fields.push("Title", sanitize::text(&self.title));
        fields.push("Collections", sanitize::text(&collections));
        fields.push("Sort key", sanitize::text(or_dash(sort_key)));

        if let Some(matched) = self.matched {
            fields.push("Matched", matched.as_str());
        }

        Record {
            title: self.seq.to_string(),
            state: Vec::new(),
            fields,
            body: sanitize::text(self.snippet.as_deref().unwrap_or_default()).into_owned(),
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
#[serde(rename_all = "kebab-case")]
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
    /// For a query bounded by date, the calendar items in scope whose
    /// occurrences a bound cut before the range's end.
    pub cut: u64,
    /// Whether the query's zone is another than the one the index read
    /// floating times and dates in, a date term reaching a calendar: its
    /// days are read in one zone over times placed in another. It leaves
    /// the answer complete.
    pub zone_differs: bool,
}

impl From<&PimdirSearchCoverage> for CoverageRow {
    fn from(coverage: &PimdirSearchCoverage) -> Self {
        let kinds = coverage.local.kinds.iter().map(|(kind, counts)| {
            let row = UnreadRow {
                bodiless: counts.bodiless,
                missing: counts.missing,
                unparseable: counts.unparseable,
                encrypted: counts.encrypted,
            };
            (kind.clone(), row)
        });

        Self {
            complete: coverage.is_complete(),
            local: LocalRow {
                kinds: kinds.collect(),
                reads: ReadsRow {
                    kinds: coverage.local.reads.kinds.iter().cloned().collect(),
                    content: coverage.local.reads.content,
                    mark: coverage.local.reads.mark,
                },
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
            cut: coverage.cut,
            zone_differs: coverage.zone_differs,
        }
    }
}

impl fmt::Display for CoverageRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let zone = " - the query's zone is not the one the index read floating times in";
        if self.complete {
            writeln!(f, "Coverage: complete")?;
            if self.zone_differs {
                writeln!(f, "{zone}")?;
            }
            return Ok(());
        }
        writeln!(f, "Coverage: partial")?;
        // NOTE: a count the query's terms do not read is left out, as it
        // leaves the answer complete.
        let reads = PimdirSearchReads::from(&self.local.reads);
        for (kind, counts) in &self.local.kinds {
            let read = reads.kinds.contains(kind);
            let bodiless = reads.counts_bodiless(kind);
            let kind = sanitize::text(kind);
            if counts.bodiless > 0 && bodiless {
                writeln!(
                    f,
                    " - {} {kind} item(s) without a body, searched by summary",
                    counts.bodiless
                )?;
            }
            if counts.missing > 0 && read {
                writeln!(
                    f,
                    " - {} {kind} item(s) whose body could not be read, searched by summary",
                    counts.missing
                )?;
            }
            if counts.unparseable > 0 && read {
                writeln!(
                    f,
                    " - {} {kind} item(s) whose body could not be parsed, searched by summary",
                    counts.unparseable
                )?;
            }
            if counts.encrypted > 0 && self.local.reads.content {
                writeln!(
                    f,
                    " - {} {kind} item(s) encrypted, searched by their headers",
                    counts.encrypted
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
                let since = sanitize::text(since);
                writeln!(f, " - items before {since} are not all held")?;
            }
            if let Some(until) = until {
                let until = sanitize::text(until);
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
        if self.cut > 0 {
            writeln!(
                f,
                " - {} calendar item(s) cut by the occurrence bounds before the range ends",
                self.cut
            )?;
        }
        if self.zone_differs {
            writeln!(f, "{zone}")?;
        }
        Ok(())
    }
}

/// The local axis.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LocalRow {
    /// The bodies out of sight among the query's collections, per
    /// collection media type, a kind counting nothing left out.
    pub kinds: BTreeMap<String, UnreadRow>,
    /// The bodies the query's terms read, which says whether a count of
    /// `kinds` leaves the answer incomplete.
    pub reads: ReadsRow,
    /// Live items whose last change the index has not folded in yet.
    pub unindexed: u64,
    /// The store's changes the index has not folded in, store-wide; an
    /// answer behind the store is never complete.
    pub behind: u64,
}

/// One kind's bodies out of sight, a body filed twice counted once per
/// placement.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UnreadRow {
    /// Live items holding no body, searched by their summary: counted
    /// against a query whose terms read a body of the kind and, for
    /// mail, against one holding `has:attachment`.
    pub bodiless: u64,
    /// Placements whose body the indexer could not read: counted against
    /// a query whose terms read a body of the kind.
    pub missing: u64,
    /// Placements whose body no parser accepted: counted likewise.
    pub unparseable: u64,
    /// Placements whose body is encrypted, searched by their headers:
    /// counted against a query whose text reaches the body or the
    /// attachments.
    pub encrypted: u64,
}

/// The bodies a query's terms read.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReadsRow {
    /// The collection media types whose bodies a term reads.
    pub kinds: Vec<String>,
    /// Whether a text term is matched in the body or the attachment
    /// field: free text, `body:`, `attachment:` or `mimetype:`.
    pub content: bool,
    /// Whether the query holds `has:attachment`, which a message holding
    /// no body answers by a guess: mail's bodiless items then count.
    pub mark: bool,
}

impl From<&ReadsRow> for PimdirSearchReads {
    fn from(reads: &ReadsRow) -> Self {
        Self {
            kinds: reads.kinds.iter().cloned().collect(),
            content: reads.content,
            mark: reads.mark,
        }
    }
}

/// The temporal axis.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case", tag = "kind")]
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
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum RemoteRow {
    /// Only the local store was searched.
    LocalOnly,
    /// A server-side search was merged in.
    Augmented {
        /// Whether the server answered every match.
        exhaustive: bool,
    },
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
        // NOTE: line by line, so a multi-line text keeps its breaks.
        let lines = |text: &str| -> String {
            let lines = text.trim_end().lines().map(sanitize::text);
            lines.collect::<Vec<_>>().join("\n")
        };
        let mut fields = Fields::default();

        fields.push("Seq", self.seq);

        if let Some(account) = &self.account {
            fields.push("Account", sanitize::text(account));
        }

        fields.push("Kind", sanitize::text(&self.kind));
        fields.push("Title", sanitize::text(&self.title));

        for placement in &self.placements {
            let collection = sanitize::text(&placement.collection);
            let link_id = sanitize::text(&placement.link_id);
            let level = &placement.level;
            fields.push("Placement", format!("{collection} as {link_id} ({level})"));
        }

        fields.push("Read from", self.source.as_str());
        fields.push("Status", sanitize::text(&self.status));

        let texts = [
            ("People", &self.fields.people),
            ("Attachments", &self.fields.attachment),
            ("Place", &self.fields.place),
            ("Org", &self.fields.org),
        ];

        for (label, text) in texts {
            if !text.is_empty() {
                fields.push(label, lines(text));
            }
        }

        write!(f, "{fields}")?;

        for text in [&self.fields.body, &self.fields.note] {
            if !text.is_empty() {
                writeln!(f, "\n{}", lines(text))?;
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

#[cfg(test)]
mod tests {
    use crate::cli::search::*;

    /// A complete coverage.
    fn complete() -> CoverageRow {
        CoverageRow {
            complete: true,
            local: LocalRow {
                kinds: BTreeMap::new(),
                reads: ReadsRow {
                    kinds: Vec::new(),
                    content: false,
                    mark: false,
                },
                unindexed: 0,
                behind: 0,
            },
            temporal: TemporalRow::Full,
            remote: RemoteRow::LocalOnly,
            horizon_left: false,
            cut: 0,
            zone_differs: false,
        }
    }

    /// One kind's counts.
    fn unread(bodiless: u64, missing: u64, unparseable: u64, encrypted: u64) -> UnreadRow {
        UnreadRow {
            bodiless,
            missing,
            unparseable,
            encrypted,
        }
    }

    #[test]
    fn a_coverage_prints_the_counts_its_query_reads() {
        let mail = String::from("message/rfc822");
        let mut coverage = complete();
        coverage
            .local
            .kinds
            .insert(mail.clone(), unread(5, 2, 1, 3));
        coverage.zone_differs = true;
        assert_eq!(
            coverage.to_string(),
            "Coverage: complete\n \
             - the query's zone is not the one the index read floating times in\n",
            "counts no term reads, and a zone reported apart"
        );

        // NOTE: `has:attachment` reads no message's body and still counts
        // the messages holding none, their mark being a guess.
        coverage.complete = false;
        coverage.zone_differs = false;
        coverage.local.reads.mark = true;
        assert_eq!(
            coverage.to_string(),
            "Coverage: partial\n \
             - 5 message/rfc822 item(s) without a body, searched by summary\n"
        );

        coverage.local.reads.mark = false;
        coverage.local.reads.kinds = vec![mail];
        assert_eq!(
            coverage.to_string(),
            "Coverage: partial\n \
             - 5 message/rfc822 item(s) without a body, searched by summary\n \
             - 2 message/rfc822 item(s) whose body could not be read, searched by summary\n \
             - 1 message/rfc822 item(s) whose body could not be parsed, searched by summary\n"
        );

        coverage.local.reads.content = true;
        coverage.local.behind = 4;
        assert_eq!(
            coverage.to_string(),
            "Coverage: partial\n \
             - 5 message/rfc822 item(s) without a body, searched by summary\n \
             - 2 message/rfc822 item(s) whose body could not be read, searched by summary\n \
             - 1 message/rfc822 item(s) whose body could not be parsed, searched by summary\n \
             - 3 message/rfc822 item(s) encrypted, searched by their headers\n \
             - the index is 4 change(s) behind the store\n"
        );
    }

    /// The placement of an item in `collection`.
    fn placement(collection: &str) -> PlacementRow {
        PlacementRow {
            collection: String::from(collection),
            link_id: String::from("a@example.org"),
            flags: Some(Vec::new()),
            object: None,
            level: String::from("full"),
            sort_key: String::from("2026-08-01T10:00:00Z"),
        }
    }

    /// A mail hit dated 2026-08-01 in INBOX, without account or snippet.
    fn hit(seq: i64, title: &str) -> HitRow {
        HitRow {
            account: None,
            seq,
            kind: String::from("mail"),
            title: String::from(title),
            sort_key: String::from("2026-08-01T10:00:00Z"),
            matched: None,
            snippet: None,
            placements: vec![placement("INBOX")],
        }
    }

    #[test]
    fn a_search_prints_one_record_per_hit_its_snippet_as_the_body() {
        let mut invoice = hit(3, "Invoice \u{202e}fdp.exe");
        invoice.account = Some(String::from("work"));
        invoice.matched = Some(Matched::Body);
        invoice.snippet = Some(String::from("…pay \u{1b}]52;c;aGk=\u{7} now…"));
        invoice.placements.push(placement("Archives/2026"));

        let mut alice = hit(7, "Alice");
        alice.account = Some(String::from("work"));
        alice.kind = String::from("contact");
        alice.sort_key = String::from("alice");
        alice.placements = vec![placement("contacts")];

        // NOTE: a kind the query's text term reads, so its count prints.
        let kind = String::from("text/vcard\u{1b}[2J");
        let mut coverage = complete();
        coverage.complete = false;
        coverage.local.reads.kinds = vec![kind.clone()];
        coverage.local.kinds.insert(kind, unread(1, 0, 0, 0));

        let output = SearchOutput {
            query: String::from("invoice"),
            hits: vec![invoice, alice],
            next_page: Some(String::from("2")),
            coverage,
        };

        let expected = "\
3
Account:      work
Kind:         mail
Title:        Invoice \u{fffd}fdp.exe
Collections:  INBOX, Archives/2026
Sort key:     2026-08-01T10:00:00Z
Matched:      body

    …pay \u{fffd}]52;c;aGk=\u{fffd} now…

7
Account:      work
Kind:         contact
Title:        Alice
Collections:  contacts
Sort key:     alice

More hits follow: pass --page-token 2
Coverage: partial
 - 1 text/vcard\u{fffd}[2J item(s) without a body, searched by summary
";
        assert_eq!(output.to_string(), expected);
    }

    #[test]
    fn threads_print_one_record_each_their_hits_as_the_body() {
        let mut alice = hit(7, "Alice");
        alice.kind = String::from("contact");

        let output = SearchThreadsOutput {
            query: String::from("report or alice"),
            threads: vec![
                ThreadRow {
                    account: None,
                    thread: Some(String::from("a@example.org")),
                    count: 3,
                    first: Some(String::from("2026-08-01T10:00:00Z")),
                    last: Some(String::from("2026-08-02T10:00:00Z")),
                    hits: vec![hit(1, "Report"), hit(14, "Re: \u{1b}[31mReport")],
                },
                ThreadRow {
                    account: None,
                    thread: None,
                    count: 1,
                    first: None,
                    last: None,
                    hits: vec![alice],
                },
            ],
            next_page: None,
            coverage: complete(),
        };

        let expected = "\
a@example.org
Messages:  3
First:     2026-08-01T10:00:00Z
Last:      2026-08-02T10:00:00Z

    1   mail  Report
    14  mail  Re: \u{fffd}[31mReport

-
Messages:  1
First:     -
Last:      -

    7  contact  Alice

Coverage: complete
";
        assert_eq!(output.to_string(), expected);
    }

    #[test]
    fn a_thread_prints_its_fields_then_its_hits() {
        let mut first = hit(1, "Report");
        first.snippet = Some(String::from("a words"));

        let output = ThreadRow {
            account: None,
            thread: Some(String::from("a@example.org")),
            count: 2,
            first: Some(String::from("2026-08-01T10:00:00Z")),
            last: Some(String::from("2026-08-01T10:00:00Z")),
            hits: vec![first, hit(4, "Re: Report\u{7}")],
        };

        let expected = "\
Thread:    a@example.org
Messages:  2
First:     2026-08-01T10:00:00Z
Last:      2026-08-01T10:00:00Z

1
Kind:         mail
Title:        Report
Collections:  INBOX
Sort key:     2026-08-01T10:00:00Z

    a words

4
Kind:         mail
Title:        Re: Report\u{fffd}
Collections:  INBOX
Sort key:     2026-08-01T10:00:00Z
";
        assert_eq!(output.to_string(), expected);
    }

    #[test]
    fn a_tag_prints_one_record_per_hit_and_a_field_per_placement() {
        let placement = |collection: &str, status, missing: &[&str]| TagPlacementRow {
            collection: String::from(collection),
            link_id: String::from("c@example.org"),
            flags: Some(vec![String::from("\\Seen")]),
            next: Some(vec![String::from("\\Flagged"), String::from("\\Seen")]),
            status,
            source: Some(String::from("local")),
            sources: vec![String::from("local")],
            syncs: true,
            missing: missing.iter().map(|name| name.to_string()).collect(),
            row: None,
            handle: None,
        };

        let output = TagOutput {
            query: String::from("invoice"),
            dry_run: true,
            add: vec![String::from("\\Flagged")],
            remove: Vec::new(),
            hits: vec![TagHitRow {
                account: None,
                seq: 3,
                kind: String::from("mail"),
                title: String::from("Invoice \u{202e}fdp.exe"),
                placements: vec![
                    placement("INBOX", TagStatus::Planned, &[]),
                    placement("Old\u{1b}[2J", TagStatus::Unsupported, &["flags"]),
                ],
            }],
            coverage: complete(),
        };

        let expected = "\
3
Kind:       mail
Title:      Invoice \u{fffd}fdp.exe
Placement:  INBOX: \\Flagged \\Seen (planned)
Placement:  Old\u{fffd}[2J: \\Flagged \\Seen (unsupported)
            not staged: its source supports no flags

1 placement(s) planned
1 placement(s) unsupported
Coverage: complete
";
        assert_eq!(output.to_string(), expected);
    }

    #[test]
    fn show_prints_fields_then_the_text_line_by_line() {
        let output = ShowOutput {
            account: None,
            seq: 1,
            kind: String::from("mail"),
            title: String::from("Hi\u{1b}[2J"),
            source: Matched::Body,
            status: String::from("ok"),
            placements: vec![placement("INBOX")],
            fields: FieldsRow {
                title: String::from("Hi\u{1b}[2J"),
                people: String::from("Alice <alice@example.org>\nbob@example.org"),
                body: String::from("Hello\n\u{1b}]52;c;aGk=\u{7}\n\nBye\n"),
                attachment: String::new(),
                place: String::new(),
                org: String::new(),
                note: String::new(),
                from: String::new(),
                to: String::new(),
                cc: String::new(),
                bcc: String::new(),
                email: String::new(),
                organizer: String::new(),
                attendee: String::new(),
            },
            chunks: Vec::new(),
        };

        let expected = "\
Seq:        1
Kind:       mail
Title:      Hi\u{fffd}[2J
Placement:  INBOX as a@example.org (full)
Read from:  body
Status:     ok
People:     Alice <alice@example.org>
            bob@example.org

Hello
\u{fffd}]52;c;aGk=\u{fffd}

Bye
";
        assert_eq!(output.to_string(), expected);
    }
}
