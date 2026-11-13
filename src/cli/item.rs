//! The `item` verb group: list, show, export, restore, purge and erase.
//!
//! None of it interprets content. An item prints as its public `seq`,
//! its cross-source link id, its flags, its detail level, its body hash
//! and its summary row as stored; a body leaves as the raw bytes that
//! went in. Rendering those is a per-kind consumer's job.

use std::{
    fmt,
    fs::File,
    io::{self, Read, Write, stdout},
    path::PathBuf,
};

use anyhow::{Result, bail};
use clap::{ArgGroup, Args, Subcommand};
use io_pimdir::{
    client::{
        reader::{PimdirItem, PimdirReader},
        search::PimdirIndex,
    },
    codec::PimdirAction,
    hub::{PimdirBinding, PimdirSourceId},
    object::PimdirHash,
    placement::{PimdirFlags, PimdirLevel},
};
use log::warn;
use pimalaya_cli::{
    fields::Fields,
    output::Bytes,
    printer::Printer,
    records::{Record, Records},
    sanitize,
};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

use crate::cli::{
    StoreFlags, bytes, confirm,
    error::{ErrorCode, partial, report},
    now, or_dash, summary,
};

/// The default page size of an item listing.
const DEFAULT_LIMIT: usize = 50;

/// Inspect and repair the items of a collection.
///
/// Items are shown by their public `seq`, the small store-global id every
/// consumer already holds. A retained item (one a delete left behind) is hidden
/// from the ordinary listing and shows up under `--retained`.
#[derive(Debug, Subcommand)]
pub enum ItemCommand {
    /// List a collection's items, live ones by default.
    List(ItemListCommand),
    /// Show one item by its public id.
    Show(ItemShowCommand),
    /// Write one item's body to stdout or to a file.
    Export(ItemExportCommand),
    /// Bring a retained item back.
    Restore(ItemRestoreCommand),
    /// Destroy retained items for good.
    Purge(ItemPurgeCommand),
    /// Erase one item's bytes from the store's files.
    Erase(ItemEraseCommand),
}

impl ItemCommand {
    /// Runs the selected subcommand.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        match self {
            Self::List(cmd) => cmd.execute(printer, store),
            Self::Show(cmd) => cmd.execute(printer, store),
            Self::Export(cmd) => cmd.execute(printer, store),
            Self::Restore(cmd) => cmd.execute(printer, store),
            Self::Purge(cmd) => cmd.execute(printer, store),
            Self::Erase(cmd) => cmd.execute(printer, store),
        }
    }
}

/// List a collection's items, one page at a time.
///
/// The listing is keyset-paged: it never truncates silently, and tells you the
/// token to pass to `--page-token` for the next page. Live items come in the
/// kind's own order (newest mail first, contacts and calendars ascending) with
/// the summary's title, retained ones by public id.
#[derive(Debug, Args)]
pub struct ItemListCommand {
    /// Collection to list, as shown by `collection list`.
    #[arg(value_name = "COLLECTION")]
    pub collection: String,

    /// List the retained (soft-deleted) items instead of the live ones.
    #[arg(long)]
    pub retained: bool,

    /// Resume a listing at the given page.
    ///
    /// The token is the `nextPage` value a previous listing printed, given
    /// back as it is.
    #[arg(long = "page-token", alias = "after", value_name = "TOKEN")]
    pub page_token: Option<String>,

    /// Maximum number of items to print.
    #[arg(long, short = 'n', value_name = "COUNT", default_value_t = DEFAULT_LIMIT)]
    pub limit: usize,
}

impl ItemListCommand {
    /// Prints one page of the collection's items.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let store = store.read()?;
        let limit = self.limit.max(1);
        // NOTE: one more than asked, so the page knows whether it is the
        // last without a second query.
        let probe = limit.saturating_add(1);

        let mut rows: Vec<ItemRow> = if self.retained {
            let after = match &self.page_token {
                None => None,
                Some(token) => Some(token.parse::<i64>().map_err(|_| invalid_token(token))?),
            };
            store
                .list_retained(&self.collection, after, probe)
                .map_err(report)?
                .into_iter()
                .map(|item| ItemRow::new(&self.collection, &item))
                .collect()
        } else {
            // NOTE: the cursor is the pair a sort-key page is total on,
            // spelled `seq:sort_key` since a seq carries no colon.
            let after = match &self.page_token {
                None => None,
                Some(token) => {
                    let (seq, key) = token.split_once(':').ok_or_else(|| invalid_token(token))?;
                    let seq: i64 = seq.parse().map_err(|_| invalid_token(token))?;
                    Some((key.to_string(), seq))
                }
            };
            store
                .list_summaries(
                    &self.collection,
                    after.as_ref().map(|(key, seq)| (key.as_str(), *seq)),
                    probe,
                )
                .map_err(report)?
                .into_iter()
                .map(|item| ItemRow::new(&self.collection, &item))
                .collect()
        };

        let truncated = rows.len() > limit;
        rows.truncate(limit);
        let next_page = truncated
            .then(|| rows.last().map(|row| row.cursor(self.retained)))
            .flatten();

        printer.out(ItemsOutput {
            collection: self.collection,
            retained: self.retained,
            items: rows,
            next_page,
        })
    }
}

/// The failure of a page token no listing printed.
fn invalid_token(token: &str) -> anyhow::Error {
    ErrorCode::PageTokenInvalid.error(format!(
        "--page-token takes the nextPage a listing printed, got {token:?}"
    ))
}

/// Show one item by its public id, across every collection holding it.
///
/// A message filed in two mailboxes shares one `seq`, so this prints one
/// placement per collection, retained placements included, each with its
/// summary row and addresses as stored, followed by the bindings the sources
/// hold it under. Nothing here parses a body.
#[derive(Debug, Args)]
pub struct ItemShowCommand {
    /// Public id of the item (`seq`).
    #[arg(value_name = "SEQ")]
    pub seq: i64,

    /// Restrict the lookup to this collection.
    #[arg(long, short = 'c', value_name = "COLLECTION")]
    pub collection: Option<String>,
}

impl ItemShowCommand {
    /// Prints every placement of the item.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let read = store.read()?;
        let found = locate(&read, self.seq, self.collection.as_deref())?;

        if found.is_empty() {
            return Err(ErrorCode::ItemNotFound
                .error(format!("no item with seq {} in this store", self.seq)));
        }

        let mut placements = Vec::with_capacity(found.len());
        for found in found {
            let mut item = found.row();
            item.size = item
                .object
                .as_deref()
                .and_then(|hash| read.object_size(hash).ok())
                .flatten();

            let bindings = read
                .item_bindings(&item.collection, &item.link_id)
                .map_err(report)?
                .into_iter()
                .map(|(source, binding)| BindingRow::new(&source, &binding))
                .collect();

            placements.push(ItemPlacement { item, bindings });
        }

        printer.out(ItemShowOutput {
            seq: self.seq,
            placements,
        })
    }
}

/// Write one item's body out, byte for byte.
///
/// The bytes are the ones the store holds, with no decoding, no re-encoding and
/// no rendering. Without `--output` they go to stdout, as they are, or under
/// `--json` as base64 `data` with their `size` and the collection's media type
/// as `mime`.
#[derive(Debug, Args)]
pub struct ItemExportCommand {
    /// Public id of the item (`seq`).
    #[arg(value_name = "SEQ")]
    pub seq: i64,

    /// Restrict the lookup to this collection.
    #[arg(long, short = 'c', value_name = "COLLECTION")]
    pub collection: Option<String>,

    /// Write the body to this file instead of stdout.
    #[arg(long, short = 'o', value_name = "PATH")]
    pub output: Option<PathBuf>,
}

impl ItemExportCommand {
    /// Streams the item's body to stdout or to the given file.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let read = store.read()?;
        let found = one(
            locate(&read, self.seq, self.collection.as_deref())?,
            self.seq,
        )?;
        let Some(hash) = found.object() else {
            return Err(ErrorCode::BodyAbsent.error(format!(
                "seq {} holds no body in {} (its detail level is {})",
                self.seq,
                found.collection,
                level_name(found.level())
            )));
        };

        // NOTE: read whole and checked before a byte is served (STORAGE
        // §7): a body failing its name is missing.
        let blobs = read.blobs();
        let Some(mut reader) = blobs.verified(&PimdirHash(hash.clone()))? else {
            return Err(ErrorCode::BodyMissing.error(format!(
                "the body of seq {} is missing from the blob store or fails its name (hash {hash}); run `pimdir check`",
                self.seq
            )));
        };

        let hash = hash.clone();
        let collection = found.collection.clone();

        match self.output {
            // NOTE: under JSON the body travels as data, the collection's
            // declared media type naming it.
            None if printer.is_json() => {
                let mut data = Vec::new();
                reader.read_to_end(&mut data)?;
                let mime = read
                    .collection_kind(&collection)
                    .map_err(report)?
                    .filter(|kind| !kind.is_empty())
                    .unwrap_or_else(|| String::from("application/octet-stream"));
                printer.out_bytes(Bytes { data, mime })
            }
            None => {
                let mut out = stdout().lock();
                io::copy(&mut reader, &mut out)?;
                out.flush()?;
                Ok(())
            }
            Some(path) => {
                let mut file = File::create(&path)?;
                let size = io::copy(&mut reader, &mut file)?;
                file.flush()?;
                printer.out(ItemExportOutput {
                    seq: self.seq,
                    collection,
                    hash,
                    size,
                    path,
                })
            }
        }
    }
}

/// Bring a retained item back into its collection.
///
/// The retained row still holds the item's link id, flags and body, so the
/// restore rebuilds it from those, as an `add` action appended to the store's
/// queue; the owner derives the summary from the body again. The action
/// applies straight away when the owner role is free; when a sync holds it,
/// the action stays queued and applies at the next drain. Either way nothing
/// is lost.
///
/// Exits 2 once the result is printed when the drain ran and the item is
/// still not live: the store refused the action, which `pimdir queue list
/// --parked` shows.
#[derive(Debug, Args)]
pub struct ItemRestoreCommand {
    /// Public id of the retained item (`seq`).
    #[arg(value_name = "SEQ")]
    pub seq: i64,

    /// Restrict the lookup to this collection.
    #[arg(long, short = 'c', value_name = "COLLECTION")]
    pub collection: Option<String>,
}

impl ItemRestoreCommand {
    /// Enqueues the restore, then applies it if the owner role is free.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let read = store.read()?;
        let found = one(
            locate(&read, self.seq, self.collection.as_deref())?,
            self.seq,
        )?;
        let item = &found.item;
        if item.retention.is_none() {
            return Err(ErrorCode::ItemLive.error(format!(
                "seq {} is already live in {}: only a retained item can be restored",
                self.seq, found.collection
            )));
        }
        // NOTE: resolved before the enqueue, so an unresolvable write
        // source fails while nothing has been appended yet.
        let source = store.write_source(&read)?;
        drop(read);

        // NOTE: a payload carries a known set (STORAGE §15.3), and an add
        // starts from an empty one, which a set never read restores as.
        let action = PimdirAction::Add {
            link_id: Some(item.link_id.clone()),
            flags: match item.flags.is_unknown() {
                true => PimdirFlags::default(),
                false => item.flags.clone(),
            },
            object: item.object.clone(),
            supersedes: None,
        };
        // NOTE: no size, the body being already indexed: the retained row
        // kept its object alive, which is the point of retention.
        let id = store
            .producer()?
            .enqueue(&found.collection, &action, None)
            .map_err(report)?
            .handle;

        // NOTE: the drain reports what it did to the whole collection's
        // queue, so the item itself is the only trustworthy proof that
        // this action landed: it is live again, or it is not. An owner
        // running meanwhile leaves the action queued for itself.
        let status = match store.owner_if_free()? {
            None => RestoreStatus::Queued,
            Some(owner) => {
                let mut owner = owner.for_source(source);
                match owner.drain() {
                    Err(io_pimdir::client::PimdirError::Busy) => RestoreStatus::Queued,
                    Err(err) => return Err(report(err)),
                    Ok(_) => match owner
                        .get_item(&found.collection, self.seq)
                        .map_err(report)?
                    {
                        Some(_) => RestoreStatus::Applied,
                        None => RestoreStatus::Refused,
                    },
                }
            }
        };

        printer.out(ItemRestoreOutput {
            seq: self.seq,
            collection: found.collection,
            link_id: item.link_id.0.clone(),
            row: id.id,
            handle: id.to_string(),
            queued: status == RestoreStatus::Queued,
            status,
        })?;

        match status {
            RestoreStatus::Refused => Err(partial()),
            RestoreStatus::Applied | RestoreStatus::Queued => Ok(()),
        }
    }
}

/// Destroy retained items for good, releasing their bodies.
///
/// This is the only true delete a pimdir store has, and it cannot be undone:
/// the row goes, and the body it held is reclaimed by `pimdir gc` once nothing
/// else references it.
/// Give one `seq`, or `--older-than` to sweep everything retained before a
/// point in time, or `--all` to empty the trash. Destructive, so it asks first
/// unless `--yes` is passed.
#[derive(Debug, Args)]
#[command(group(ArgGroup::new("target").required(true).args(["seq", "older_than", "all"])))]
pub struct ItemPurgeCommand {
    /// Public id of the retained item to destroy (`seq`).
    #[arg(value_name = "SEQ")]
    pub seq: Option<i64>,

    /// Destroy every item retained longer than this (`90d`, `12h`, `2weeks`).
    #[arg(long, value_name = "DURATION")]
    pub older_than: Option<humantime::Duration>,

    /// Destroy every retained item in the store.
    #[arg(long)]
    pub all: bool,

    /// Restrict a `seq` purge to this collection.
    #[arg(long, short = 'c', value_name = "COLLECTION")]
    pub collection: Option<String>,

    /// Do not ask for confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

impl ItemPurgeCommand {
    /// Purges one item, or every item retained before the computed cutoff.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        if let Some(seq) = self.seq {
            return self.purge_one(printer, store, seq);
        }

        // NOTE: `--all` is `--older-than 0`: nothing can be retained in
        // the future, so "before now" is "everything".
        let cutoff = match self.older_than {
            Some(age) => crate::cli::cutoff(*age),
            None => now(),
        };

        let preview = store
            .read()
            .and_then(|read| read.retained_before(&cutoff).map_err(report))
            .map_err(|err| warn!("cannot preview what this purge would destroy: {err}"))
            .ok();

        let question = match preview {
            Some((items, size)) => format!(
                "Destroy {items} retained item(s) for good, releasing up to {}?",
                bytes(size)
            ),
            None => format!("Destroy every item retained before {cutoff} for good?"),
        };
        confirm(printer, self.yes, &question)?;

        let purged = store
            .owner()?
            .purge_retained_before(&cutoff)
            .map_err(report)?;

        printer.out(ItemPurgeOutput {
            cutoff: Some(cutoff),
            count: purged.items,
        })
    }

    /// Purges a single retained item, refusing a live one.
    fn purge_one(&self, printer: &mut impl Printer, store: &StoreFlags, seq: i64) -> Result<()> {
        let read = store.read()?;
        let found = one(locate(&read, seq, self.collection.as_deref())?, seq)?;
        let Some(retention) = &found.item.retention else {
            return Err(ErrorCode::ItemLive.error(format!(
                "seq {seq} is live in {}: purge only destroys retained items, remove it first",
                found.collection
            )));
        };
        let size = retention.size.unwrap_or(0);
        drop(read);

        confirm(
            printer,
            self.yes,
            &format!(
                "Destroy retained item {seq} ({}) in {} for good?",
                bytes(size),
                found.collection
            ),
        )?;

        let purged = store
            .owner()?
            .purge(&found.collection, seq)
            .map_err(report)?;

        if !purged {
            let collection = &found.collection;
            bail!("seq {seq} was not purged: it is no longer retained in {collection}");
        }

        printer.out(ItemPurgeOutput {
            cutoff: None,
            count: 1,
        })
    }
}

/// Erase one item's bytes from the store's files.
///
/// A purge removes a row, and SQLite keeps what it freed in the database
/// files until it reuses the space. An erase runs the whole procedure: the
/// purge of the retained item, or the delete of a live one no source binds,
/// the collector, the search index's refresh and the merge of its full-text
/// segments when the store keeps one, then a checkpoint truncating both
/// write-ahead logs. The item's bytes are then in neither database nor in
/// objects/, unless another item holds the same body; a backup, quarantine/
/// and the filesystem's own copies stay. A live item a source syncs is
/// removed through that source first. Takes the owner role and asks first
/// unless `--yes` is passed.
#[derive(Debug, Args)]
pub struct ItemEraseCommand {
    /// Public id of the item to erase (`seq`).
    #[arg(value_name = "SEQ")]
    pub seq: i64,

    /// Restrict the lookup to this collection.
    #[arg(long, short = 'c', value_name = "COLLECTION")]
    pub collection: Option<String>,

    /// Do not ask for confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

impl ItemEraseCommand {
    /// Erases the item, then the index's copy of it when one is kept.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let read = store.read()?;
        let found = one(
            locate(&read, self.seq, self.collection.as_deref())?,
            self.seq,
        )?;
        if found.item.retention.is_none() {
            let bound = read
                .item_bindings(&found.collection, found.item.link_id.as_str())
                .map_err(report)?;
            if !bound.is_empty() {
                return Err(ErrorCode::ItemLive.error(format!(
                    "seq {} is live in {} and a source syncs it: remove it first, then erase it once retained",
                    self.seq, found.collection
                )));
            }
        }
        drop(read);

        confirm(
            printer,
            self.yes,
            &format!(
                "Erase item {} in {} from the store's files for good?",
                self.seq, found.collection
            ),
        )?;

        let erased = store
            .owner()?
            .erase(&found.collection, self.seq)
            .map_err(report)?;
        if !erased.erased {
            bail!(
                "seq {} was not erased: it is no longer retained nor unbound in {}",
                self.seq,
                found.collection
            );
        }

        let index = store.dir().join("index.db").is_file();
        if index {
            PimdirIndex::open(store.dir())
                .and_then(|mut index| index.erase())
                .map_err(report)?;
        }

        printer.out(ItemEraseOutput {
            seq: self.seq,
            collection: found.collection,
            blobs: erased.collected.blobs,
            bytes: erased.collected.bytes,
            index,
        })
    }
}

/// A located item together with the collection holding it. Retained or live is
/// the item's own `retention`, not a shape of its own.
struct Found {
    collection: String,
    item: PimdirItem,
}

impl Found {
    /// The item's body hash, when it has one.
    fn object(&self) -> Option<&String> {
        self.item.object.as_ref().map(|hash| &hash.0)
    }

    /// The item's detail level.
    fn level(&self) -> PimdirLevel {
        self.item.level
    }

    /// The item as a printable row.
    fn row(&self) -> ItemRow {
        ItemRow::new(&self.collection, &self.item)
    }
}

/// Finds every placement of `seq`, live or retained, in one collection or in
/// the whole store.
fn locate(store: &PimdirReader, seq: i64, collection: Option<&str>) -> Result<Vec<Found>> {
    let collections: Vec<String> = match collection {
        Some(collection) => vec![collection.to_string()],
        None => store
            .list_collections()
            .map_err(report)?
            .into_iter()
            .map(|collection| collection.id)
            .collect(),
    };

    let mut found = Vec::new();
    for collection in collections {
        if let Some(item) = store.get_item(&collection, seq).map_err(report)? {
            found.push(Found { collection, item });
            continue;
        }
        if let Some(item) = retained(store, &collection, seq)? {
            found.push(Found { collection, item });
        }
    }

    Ok(found)
}

/// One retained item by its public id, or `None`.
///
/// The retained listing is keyset-paged on `seq` in ascending order, so asking
/// for the single row after `seq - 1` either answers with `seq` itself or
/// proves it is not retained here.
fn retained(store: &PimdirReader, collection: &str, seq: i64) -> Result<Option<PimdirItem>> {
    let page = store
        .list_retained(collection, Some(seq - 1), 1)
        .map_err(report)?;
    Ok(page.into_iter().find(|item| item.seq == seq))
}

/// Reduces a lookup to exactly one placement, or explains the ambiguity.
fn one(found: Vec<Found>, seq: i64) -> Result<Found> {
    match found.len() {
        0 => Err(ErrorCode::ItemNotFound.error(format!("no item with seq {seq} in this store"))),
        1 => Ok(found.into_iter().next().unwrap()),
        _ => {
            let collections: Vec<&str> = found.iter().map(|f| f.collection.as_str()).collect();
            Err(ErrorCode::CollectionRequired.error(format!(
                "seq {seq} is placed in several collections ({}): pick one with --collection",
                collections.join(", ")
            )))
        }
    }
}

/// The detail ladder as its lowercase name.
fn level_name(level: PimdirLevel) -> &'static str {
    match level {
        PimdirLevel::Meta => "meta",
        PimdirLevel::Full => "full",
    }
}

/// The flag set as a sorted list of raw strings, never interpreted, or
/// `None` while nothing has read them (spec §13, a `NULL` flags column).
fn flag_list(flags: &PimdirFlags) -> Option<Vec<String>> {
    Some(flags.known()?.iter().cloned().collect())
}

/// The flags as the text output prints them, joined with `,`, `-` when
/// none or never read.
fn flags_text(flags: Option<&[String]>) -> String {
    match flags.map(|flags| flags.join(",")) {
        Some(flags) if !flags.is_empty() => sanitize::text(&flags).into_owned(),
        _ => String::from("-"),
    }
}

/// One item as every listing prints it.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ItemRow {
    /// The collection holding this placement.
    pub collection: String,
    /// The item's public id.
    pub seq: i64,
    /// The cross-source link id.
    pub link_id: String,
    /// The raw flag strings, `null` while nothing has read them.
    pub flags: Option<Vec<String>>,
    /// The detail level (`meta`, `full`).
    pub level: &'static str,
    /// The body's content hash, when hydrated.
    pub object: Option<String>,
    /// The body's size, when known.
    pub size: Option<u64>,
    /// The kind's ordering key, empty while unknown.
    pub sort_key: String,
    /// What a listing shows the item as: the summary's subject, name or
    /// summary line, `null` while no summary was derived.
    pub title: Option<String>,
    /// The summary row and its addresses as stored, on the reads that join
    /// them (`item show`, and a live listing).
    pub summary: Option<Value>,
    /// When the last binding vanished, for a retained item.
    pub retained_at: Option<String>,
    /// The source whose removal retired the item.
    pub retained_by: Option<String>,
}

impl ItemRow {
    /// One item's row, live or retained.
    fn new(collection: &str, item: &PimdirItem) -> Self {
        let retention = item.retention.as_ref();
        Self {
            collection: collection.to_string(),
            seq: item.seq,
            link_id: item.link_id.0.clone(),
            flags: flag_list(&item.flags),
            level: level_name(item.level),
            object: item.object.as_ref().map(|hash| hash.0.clone()),
            size: retention.and_then(|retention| retention.size),
            sort_key: item.sort_key.clone(),
            title: item.summary.as_ref().map(|s| s.title().to_string()),
            summary: item.summary.as_ref().map(summary::json),
            retained_at: retention.and_then(|retention| retention.at.clone()),
            retained_by: retention.and_then(|retention| retention.by.clone()),
        }
    }

    /// The cursor resuming a listing after this row.
    fn cursor(&self, retained: bool) -> String {
        if retained {
            self.seq.to_string()
        } else {
            format!("{}:{}", self.seq, self.sort_key)
        }
    }
}

/// The `item list` output: one page, plus what it left out.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ItemsOutput {
    /// The listed collection.
    pub collection: String,
    /// Whether the page lists retained items.
    pub retained: bool,
    /// The page.
    pub items: Vec<ItemRow>,
    /// The token `--page-token` resumes from, `None` when the page is the
    /// last one.
    pub next_page: Option<String>,
}

impl fmt::Display for ItemsOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.items.is_empty() {
            let what = if self.retained { "retained" } else { "live" };
            return writeln!(f, "No {what} item in {}", self.collection);
        }

        let mut records = Records::default();

        for item in &self.items {
            let flags = item.flags.iter().flatten();
            let state = flags.map(|flag| sanitize::text(flag).into_owned());
            let title = item.title.as_deref().unwrap_or_default();
            let mut fields = Fields::default();

            fields.push("Level", item.level);

            if self.retained {
                let at = or_dash(item.retained_at.as_deref());
                let by = or_dash(item.retained_by.as_deref());
                fields.push("Retained at", sanitize::text(at));
                fields.push("Retained by", sanitize::text(by));
                fields.push("Link id", sanitize::text(&item.link_id));
            } else {
                let key = Some(item.sort_key.as_str()).filter(|key| !key.is_empty());
                fields.push("Sort key", sanitize::text(or_dash(key)));
            }

            records.push(Record {
                title: item.seq.to_string(),
                state: state.collect(),
                fields,
                body: sanitize::text(title).into_owned(),
            });
        }

        write!(f, "{records}")?;

        // NOTE: a page that stops short says so, or an operator would
        // mistake one page for the whole collection.
        if let Some(next) = &self.next_page {
            let next = sanitize::text(next);
            writeln!(f, "\nMore items follow: continue with --page-token {next}")?;
        }

        Ok(())
    }
}

/// One source's binding of an item.
///
/// How that source addresses it, what the last sync agreed on, and the marker
/// that says why it might have stopped moving.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BindingRow {
    /// The source holding the binding.
    pub source: String,
    /// The handle the item is addressed by there (an IMAP UID, a DAV href).
    pub handle: String,
    /// Whether a base exists at all, which its three values cannot say: a
    /// source reporting no revision, no body and no flags still agreed.
    pub base: bool,
    /// The base flag set, `null` while nothing has read them.
    pub base_flags: Option<Vec<String>>,
    /// The body the base agreed on, when it carried one.
    pub base_object: Option<String>,
    /// The revision the base agreed on, when the source reports one.
    pub base_revision: Option<String>,
    /// Whether this source and its own remote diverged.
    pub conflicted: bool,
    /// The remote revision observed when the divergence was recorded.
    pub conflict_revision: Option<String>,
    /// The diverging remote body at that revision, `null` while the
    /// upgrade pass has not supplied it yet.
    pub conflict_object: Option<String>,
    /// The shared body this source last reconciled against, `null` until
    /// it has folded once. What a cross-source divergence is measured
    /// from, where `base_object` is what a push is derived from.
    pub shared_object: Option<String>,
}

impl BindingRow {
    /// One binding's row.
    fn new(source: &PimdirSourceId, binding: &PimdirBinding) -> Self {
        Self {
            source: source.0.clone(),
            handle: binding.handle.0.clone(),
            base: binding.base.is_some(),
            base_flags: binding
                .base
                .as_ref()
                .and_then(|base| flag_list(&base.flags)),
            base_object: binding
                .base
                .as_ref()
                .and_then(|base| base.object.as_ref())
                .map(|hash| hash.0.clone()),
            base_revision: binding.base.as_ref().and_then(|base| base.revision.clone()),
            conflicted: binding.conflicted,
            conflict_revision: binding.conflict_revision.clone(),
            conflict_object: binding.conflict_object.as_ref().map(|hash| hash.0.clone()),
            shared_object: binding.shared_object.as_ref().map(|hash| hash.0.clone()),
        }
    }
}

/// One placement of an item, with the bindings the sources hold it under.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ItemPlacement {
    /// The item as this collection holds it.
    #[serde(flatten)]
    pub item: ItemRow,
    /// One entry per source holding this placement, ordered by source.
    pub bindings: Vec<BindingRow>,
}

/// The `item show` output: every placement of one public id.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ItemShowOutput {
    /// The public id looked up.
    pub seq: i64,
    /// One entry per collection holding it.
    pub placements: Vec<ItemPlacement>,
}

impl fmt::Display for ItemShowOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut blocks = Vec::new();

        for placement in &self.placements {
            let item = &placement.item;
            let table = item.summary.as_ref().and_then(|s| s["table"].as_str());
            let mut fields = Fields::default();

            fields.push("Seq", item.seq);
            fields.push("Collection", sanitize::text(&item.collection));
            fields.push("Link id", sanitize::text(&item.link_id));
            fields.push("Flags", flags_text(item.flags.as_deref()));
            fields.push("Level", item.level);
            fields.push("Object", or_dash(item.object.as_deref()));

            if let Some(size) = item.size {
                fields.push("Size", bytes(size));
            }

            if let Some(at) = &item.retained_at {
                let by = or_dash(item.retained_by.as_deref());
                fields.push("Retained at", sanitize::text(at));
                fields.push("Retained by", sanitize::text(by));
            }

            fields.push("Summary", or_dash(table));
            blocks.push(fields);

            if let Some(summary) = &item.summary {
                blocks.push(summary::fields(summary));
            }

            for binding in &placement.bindings {
                let mut fields = Fields::default();
                fields.push("Binding", sanitize::text(&binding.source));
                fields.push("Handle", sanitize::text(&binding.handle));

                if binding.base {
                    let object = or_dash(binding.base_object.as_deref());
                    let revision = or_dash(binding.base_revision.as_deref());
                    fields.push("Base object", object);
                    fields.push("Base flags", flags_text(binding.base_flags.as_deref()));
                    fields.push("Base revision", sanitize::text(revision));
                } else {
                    fields.push("Base", "none");
                }

                fields.push("Shared object", or_dash(binding.shared_object.as_deref()));

                // NOTE: printed only when it applies, so a diverged binding
                // stands out from the ordinary ones beside it.
                if binding.conflicted {
                    let revision = or_dash(binding.conflict_revision.as_deref());
                    let object = or_dash(binding.conflict_object.as_deref());
                    fields.push("Conflicted at revision", sanitize::text(revision));
                    fields.push("Conflict object", object);
                }

                blocks.push(fields);
            }
        }

        for (index, block) in blocks.iter().enumerate() {
            if index > 0 {
                writeln!(f)?;
            }

            write!(f, "{block}")?;
        }

        Ok(())
    }
}

/// The `item export --output` output.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ItemExportOutput {
    /// The exported item.
    pub seq: i64,
    /// The collection it was read from.
    pub collection: String,
    /// The body's content hash.
    pub hash: String,
    /// The size of the body written, in bytes.
    pub size: u64,
    /// Where they were written.
    pub path: PathBuf,
}

impl fmt::Display for ItemExportOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Wrote {} of item {} to {}",
            bytes(self.size),
            self.seq,
            self.path.display()
        )
    }
}

/// What became of a restore, which the text form says and the JSON tells
/// by `queued` and the exit status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreStatus {
    /// The action was applied and the item is live again.
    Applied,
    /// Another writer holds the store lock, or the store is of another
    /// revision, so the action waits in the queue for its sync engine.
    Queued,
    /// The drain ran and the item is still not live: the action was refused.
    Refused,
}

/// The `item restore` output.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ItemRestoreOutput {
    /// The restored item.
    pub seq: i64,
    /// The collection it returns to.
    pub collection: String,
    /// Its cross-source link id.
    pub link_id: String,
    /// The queue row the restore was appended as.
    pub row: i64,
    /// The row's handle, its id under the store's id and incarnation:
    /// what `queue status` and `queue cancel` check the store against.
    pub handle: String,
    /// Whether the row waits for the store's sync engine: another writer
    /// held the store lock. `false` with exit status 2 when the drain ran
    /// and the item is still not live.
    pub queued: bool,
    /// What became of the row, for the text form.
    #[serde(skip)]
    pub status: RestoreStatus,
}

impl fmt::Display for ItemRestoreOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.status {
            RestoreStatus::Applied => writeln!(
                f,
                "Restored item {} into {} (applied)",
                self.seq, self.collection
            ),
            RestoreStatus::Queued => writeln!(
                f,
                "Restore of item {} into {} queued as action {}: another writer holds the store lock, so it applies at the next sync",
                self.seq, self.collection, self.handle
            ),
            RestoreStatus::Refused => writeln!(
                f,
                "Restore of item {} into {} was not applied: see `pimdir queue list --parked`",
                self.seq, self.collection
            ),
        }
    }
}

/// The `item purge` output.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ItemPurgeOutput {
    /// The cutoff a time-based purge used, `None` for a single item.
    pub cutoff: Option<String>,
    /// How many items were destroyed.
    pub count: usize,
}

impl fmt::Display for ItemPurgeOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Purged {} item(s)", self.count)?;
        if self.count > 0 {
            writeln!(f, "Run `pimdir gc` to reclaim the bodies they released")?;
        }
        Ok(())
    }
}

/// The `item erase` output.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ItemEraseOutput {
    /// The public id erased.
    pub seq: i64,
    /// The collection it was in.
    pub collection: String,
    /// Blob files the collector unlinked.
    pub blobs: usize,
    /// Bytes they freed.
    pub bytes: u64,
    /// Whether the store's search index was erased too.
    pub index: bool,
}

impl fmt::Display for ItemEraseOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Erased item {} from {}, unlinking {} blob file(s) ({})",
            self.seq,
            self.collection,
            self.blobs,
            bytes(self.bytes)
        )?;
        if self.index {
            writeln!(f, "Erased it from the search index too")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::cli::item::*;

    /// A live INBOX item dated 2026-08-01 with the given title and flags.
    fn item(seq: i64, title: Option<&str>, flags: Option<&[&str]>) -> ItemRow {
        ItemRow {
            collection: String::from("INBOX"),
            seq,
            link_id: format!("{seq}@example.org"),
            flags: flags.map(|flags| flags.iter().map(|flag| flag.to_string()).collect()),
            level: "full",
            object: None,
            size: None,
            sort_key: String::from("2026-08-01T10:00:00Z"),
            title: title.map(Into::into),
            summary: None,
            retained_at: None,
            retained_by: None,
        }
    }

    #[test]
    fn a_list_prints_one_record_per_item_its_flags_as_state() {
        let live = ItemsOutput {
            collection: String::from("INBOX"),
            retained: false,
            items: vec![
                item(
                    3,
                    Some("Invoice \u{202e}fdp.exe"),
                    Some(&["\\Flagged", "$Junk\u{7}"]),
                ),
                item(1, None, None),
            ],
            next_page: Some(String::from("1:2026-08-01T10:00:00Z")),
        };

        let expected = "\
3 (\\Flagged, $Junk\u{fffd})
Level:     full
Sort key:  2026-08-01T10:00:00Z

    Invoice \u{fffd}fdp.exe

1
Level:     full
Sort key:  2026-08-01T10:00:00Z

More items follow: continue with --page-token 1:2026-08-01T10:00:00Z
";
        assert_eq!(live.to_string(), expected);

        let mut gone = item(2, Some("Lunch?"), Some(&[]));
        gone.retained_at = Some(String::from("2026-11-11T18:13:39.892Z"));
        gone.retained_by = Some(String::from("local"));

        let retained = ItemsOutput {
            collection: String::from("INBOX"),
            retained: true,
            items: vec![gone],
            next_page: None,
        };

        let expected = "\
2
Level:        full
Retained at:  2026-11-11T18:13:39.892Z
Retained by:  local
Link id:      2@example.org

    Lunch?
";
        assert_eq!(retained.to_string(), expected);
    }

    #[test]
    fn a_placement_prints_its_fields_its_summary_then_its_bindings() {
        let mut shown = item(1, Some("Hi"), Some(&["\\Seen"]));
        shown.object = Some(String::from("qlxg"));
        shown.summary = Some(json!({
            "addresses": [{"address": "alice@example.org", "name": "Alice", "role": "from"}],
            "row": {"date": null, "subject": "Hi\u{1b}[2J"},
            "table": "mail_summary",
        }));

        let binding = BindingRow {
            source: String::from("local"),
            handle: String::from("1"),
            base: true,
            base_flags: Some(vec![String::from("\\Seen")]),
            base_object: Some(String::from("qlxg")),
            base_revision: None,
            conflicted: true,
            conflict_revision: Some(String::from("7")),
            conflict_object: None,
            shared_object: Some(String::from("qlxg")),
        };

        let output = ItemShowOutput {
            seq: 1,
            placements: vec![ItemPlacement {
                item: shown,
                bindings: vec![binding],
            }],
        };

        let expected = "\
Seq:         1
Collection:  INBOX
Link id:     1@example.org
Flags:       \\Seen
Level:       full
Object:      qlxg
Summary:     mail_summary

date:     -
subject:  Hi\u{fffd}[2J
from:     Alice <alice@example.org>

Binding:                 local
Handle:                  1
Base object:             qlxg
Base flags:              \\Seen
Base revision:           -
Shared object:           qlxg
Conflicted at revision:  7
Conflict object:         -
";
        assert_eq!(output.to_string(), expected);
    }
}
