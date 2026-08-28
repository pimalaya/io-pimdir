//! The `item` verb group: list, show, export, restore and purge.
//!
//! None of it interprets content. An item prints as its public `seq`,
//! its cross-source link id, its flags, its detail level, its body hash
//! and its meta verbatim; a body leaves as the raw bytes that went in.
//! Rendering those is a per-kind consumer's job.

use std::{
    fmt,
    fs::File,
    io::{self, Write, stdout},
    path::PathBuf,
};

use anyhow::{Result, bail};
use clap::{ArgGroup, Args, Subcommand};
use io_pimdir::{PimdirItem, PimdirReader, codec::PimdirAction};
use io_replica::{
    collection::ReplicaCollectionId,
    hub::{ReplicaSourceBinding, ReplicaSourceId},
    object::ReplicaHash,
    placement::{ReplicaFlags, ReplicaLevel},
};
use log::warn;
use pimalaya_cli::{
    printer::Printer,
    table::{Cell, ContentArrangement, Table, presets::UTF8_FULL},
};
use serde::Serialize;

use crate::cli::{StoreFlags, bytes, confirm, now, or_dash, report};

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
        }
    }
}

/// List a collection's items, one page at a time.
///
/// The listing is keyset-paged: it never truncates silently, and tells you the
/// cursor to pass to `--after` for the next page. Live items are ordered by
/// link id, retained ones by public id, so `--after` takes a link id normally
/// and a `seq` with `--retained`.
#[derive(Debug, Args)]
pub struct ItemListCommand {
    /// Collection to list, as shown by `collection list`.
    #[arg(value_name = "COLLECTION")]
    pub collection: String,

    /// List the retained (soft-deleted) items instead of the live ones.
    #[arg(long)]
    pub retained: bool,

    /// Resume after this cursor: a link id, or a `seq` with `--retained`.
    #[arg(long, value_name = "CURSOR")]
    pub after: Option<String>,

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
            let after = match &self.after {
                None => None,
                Some(cursor) => Some(cursor.parse::<i64>().map_err(|_| {
                    anyhow::anyhow!("--after takes a seq with --retained, got {cursor:?}")
                })?),
            };
            let collection = ReplicaCollectionId(self.collection.clone());
            store
                .list_retained(&collection, after, probe)
                .map_err(report)?
                .into_iter()
                .map(|item| ItemRow::new(&self.collection, &item))
                .collect()
        } else {
            store
                .list_items(&self.collection, self.after.as_deref(), probe)
                .map_err(report)?
                .into_iter()
                .map(|item| ItemRow::new(&self.collection, &item))
                .collect()
        };

        let truncated = rows.len() > limit;
        rows.truncate(limit);
        let next = truncated
            .then(|| rows.last().map(|row| row.cursor(self.retained)))
            .flatten();

        printer.out(ItemsOutput {
            collection: self.collection,
            retained: self.retained,
            items: rows,
            next,
        })
    }
}

/// Show one item by its public id, across every collection holding it.
///
/// A message filed in two mailboxes shares one `seq`, so this prints one
/// placement per collection, retained placements included, each followed by the
/// bindings the sources hold it under. The meta is printed exactly as stored:
/// this tool does not parse it.
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
            bail!("no item with seq {} in this store", self.seq);
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
/// no rendering. Without `--output` they go to stdout, which is why `--json`
/// then has nothing to say and is refused.
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
        if printer.is_json() && self.output.is_none() {
            bail!("a body is raw bytes, not JSON: write it to a file with --output");
        }

        let read = store.read()?;
        let found = one(
            locate(&read, self.seq, self.collection.as_deref())?,
            self.seq,
        )?;
        let Some(hash) = found.object() else {
            bail!(
                "seq {} holds no body in {} (its detail level is {})",
                self.seq,
                found.collection,
                level_name(found.level())
            );
        };

        let blobs = store.blobs()?;
        let Some(mut reader) = blobs.reader(&ReplicaHash(hash.clone()))? else {
            bail!(
                "the body of seq {} is missing from the blob store (hash {hash}); run `pimdir check`",
                self.seq
            );
        };

        let hash = hash.clone();
        let collection = found.collection.clone();

        match self.output {
            None => {
                let mut out = stdout().lock();
                io::copy(&mut reader, &mut out)?;
                out.flush()?;
                Ok(())
            }
            Some(path) => {
                let mut file = File::create(&path)?;
                let written = io::copy(&mut reader, &mut file)?;
                file.flush()?;
                printer.out(ItemExportOutput {
                    seq: self.seq,
                    collection,
                    hash,
                    bytes: written,
                    path,
                })
            }
        }
    }
}

/// Bring a retained item back into its collection.
///
/// The retained row still holds the item's link id, flags, meta and body, so
/// the restore rebuilds it from those, as an `add` action appended to the
/// store's queue. The action is then applied straight away when the owner role
/// is free; when a sync holds it, the action stays queued and applies at the
/// next drain. Either way nothing is lost.
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
            bail!(
                "seq {} is already live in {}: only a retained item can be restored",
                self.seq,
                found.collection
            );
        }
        drop(read);

        // NOTE: resolved before the enqueue, so an unresolvable write
        // source fails while nothing has been appended yet.
        let source = store.write_source()?;

        let action = PimdirAction::Add {
            link_id: Some(item.link_id.clone()),
            flags: item.flags.clone(),
            object: item.object.clone(),
            meta: item.meta.clone(),
            handle: None,
        };
        // NOTE: no size, the body being already indexed: the retained row
        // kept its object alive, which is the point of retention.
        let id = store
            .producer()?
            .enqueue(&found.collection, &action, None, &now())
            .map_err(report)?;

        // NOTE: the drain reports what it did to the whole collection's
        // queue, so the item itself is the only trustworthy proof that
        // this action landed: it is live again, or it is not. An owner
        // running meanwhile leaves the action queued for itself.
        let status = match store.owner_if_free()? {
            None => RestoreStatus::Queued,
            Some(owner) => {
                let mut owner = owner.for_source(source);
                match owner.drain_collection(&found.collection) {
                    Err(io_pimdir::PimdirError::Busy) => RestoreStatus::Queued,
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
            action: id,
            status,
        })
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
            items: purged.items,
        })
    }

    /// Purges a single retained item, refusing a live one.
    fn purge_one(&self, printer: &mut impl Printer, store: &StoreFlags, seq: i64) -> Result<()> {
        let read = store.read()?;
        let found = one(locate(&read, seq, self.collection.as_deref())?, seq)?;
        let Some(retention) = &found.item.retention else {
            bail!(
                "seq {seq} is live in {}: purge only destroys retained items, remove it first",
                found.collection
            );
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

        let collection = ReplicaCollectionId(found.collection.clone());
        let purged = store.owner()?.purge(&collection, seq).map_err(report)?;

        if !purged {
            let collection = &found.collection;
            bail!("seq {seq} was not purged: it is no longer retained in {collection}");
        }

        printer.out(ItemPurgeOutput {
            cutoff: None,
            items: 1,
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
    fn level(&self) -> ReplicaLevel {
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
    let collection = ReplicaCollectionId(collection.to_string());
    let page = store
        .list_retained(&collection, Some(seq - 1), 1)
        .map_err(report)?;
    Ok(page.into_iter().find(|item| item.seq == seq))
}

/// Reduces a lookup to exactly one placement, or explains the ambiguity.
fn one(found: Vec<Found>, seq: i64) -> Result<Found> {
    match found.len() {
        0 => bail!("no item with seq {seq} in this store"),
        1 => Ok(found.into_iter().next().unwrap()),
        _ => {
            let collections: Vec<&str> = found.iter().map(|f| f.collection.as_str()).collect();
            bail!(
                "seq {seq} is placed in several collections ({}): pick one with --collection",
                collections.join(", ")
            )
        }
    }
}

/// The detail ladder as its lowercase name.
fn level_name(level: ReplicaLevel) -> &'static str {
    match level {
        ReplicaLevel::Probed => "probed",
        ReplicaLevel::Meta => "meta",
        ReplicaLevel::Full => "full",
    }
}

/// The flag set as a sorted list of raw strings, never interpreted, or
/// `None` while nothing has read them (spec §13, a `NULL` flags column).
fn flag_list(flags: &ReplicaFlags) -> Option<Vec<String>> {
    Some(flags.known()?.iter().cloned().collect())
}

/// One item as every listing prints it.
#[derive(Debug, Serialize)]
pub struct ItemRow {
    /// The collection holding this placement.
    pub collection: String,
    /// The item's public id.
    pub seq: i64,
    /// The cross-source link id.
    pub link_id: String,
    /// The raw flag strings, `null` while nothing has read them.
    pub flags: Option<Vec<String>>,
    /// The detail level (`probed`, `meta`, `full`).
    pub level: &'static str,
    /// The body's content hash, when hydrated.
    pub object: Option<String>,
    /// The body's size, when known.
    pub size: Option<u64>,
    /// The raw meta, verbatim.
    pub meta: Option<String>,
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
            meta: item.meta.as_ref().map(|meta| meta.0.clone()),
            retained_at: retention.map(|retention| retention.at.clone()),
            retained_by: retention.and_then(|retention| retention.by.clone()),
        }
    }

    /// The cursor resuming a listing after this row.
    fn cursor(&self, retained: bool) -> String {
        if retained {
            self.seq.to_string()
        } else {
            self.link_id.clone()
        }
    }
}

/// The `item list` output: one page, plus what it left out.
#[derive(Debug, Serialize)]
pub struct ItemsOutput {
    /// The listed collection.
    pub collection: String,
    /// Whether the page lists retained items.
    pub retained: bool,
    /// The page.
    pub items: Vec<ItemRow>,
    /// The cursor to resume from, `None` when the page is the last one.
    pub next: Option<String>,
}

impl fmt::Display for ItemsOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.items.is_empty() {
            let what = if self.retained { "retained" } else { "live" };
            return writeln!(f, "No {what} item in {}", self.collection);
        }

        let mut table = Table::new();
        let mut header = vec![
            Cell::new("SEQ"),
            Cell::new("LINK ID"),
            Cell::new("FLAGS"),
            Cell::new("LEVEL"),
            Cell::new("OBJECT"),
        ];
        if self.retained {
            header.push(Cell::new("RETAINED AT"));
            header.push(Cell::new("BY"));
        }

        table
            .load_style(UTF8_FULL)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .set_header(header);

        for item in &self.items {
            let mut row = vec![
                Cell::new(item.seq),
                Cell::new(&item.link_id),
                Cell::new(or_dash(
                    item.flags
                        .as_ref()
                        .map(|flags| flags.join(" "))
                        .filter(|f| !f.is_empty())
                        .as_deref(),
                )),
                Cell::new(item.level),
                Cell::new(or_dash(item.object.as_deref())),
            ];
            if self.retained {
                row.push(Cell::new(or_dash(item.retained_at.as_deref())));
                row.push(Cell::new(or_dash(item.retained_by.as_deref())));
            }
            table.add_row(row);
        }

        writeln!(f, "{table}")?;

        // NOTE: a page that stops short says so, or an operator would
        // mistake one page for the whole collection.
        if let Some(next) = &self.next {
            writeln!(f, "More items follow: continue with --after {next}")?;
        }

        Ok(())
    }
}

/// One source's binding of an item: how that source addresses it, what the
/// last sync agreed on, and the marker that says why it might have stopped
/// moving.
#[derive(Debug, Serialize)]
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
}

impl BindingRow {
    /// One binding's row.
    fn new(source: &ReplicaSourceId, binding: &ReplicaSourceBinding) -> Self {
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
        }
    }
}

/// One placement of an item, with the bindings the sources hold it under.
#[derive(Debug, Serialize)]
pub struct ItemPlacement {
    #[serde(flatten)]
    pub item: ItemRow,
    /// One entry per source holding this placement, ordered by source.
    pub bindings: Vec<BindingRow>,
}

/// The `item show` output: every placement of one public id.
#[derive(Debug, Serialize)]
pub struct ItemShowOutput {
    /// The public id looked up.
    pub seq: i64,
    /// One entry per collection holding it.
    pub placements: Vec<ItemPlacement>,
}

impl fmt::Display for ItemShowOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, placement) in self.placements.iter().enumerate() {
            let item = &placement.item;
            if index > 0 {
                writeln!(f)?;
            }
            writeln!(f, "Item {} in {}", item.seq, item.collection)?;
            writeln!(f, " - link id: {}", item.link_id)?;
            writeln!(
                f,
                " - flags: {}",
                or_dash(
                    item.flags
                        .as_ref()
                        .map(|flags| flags.join(" "))
                        .filter(|flags| !flags.is_empty())
                        .as_deref()
                )
            )?;
            writeln!(f, " - level: {}", item.level)?;
            writeln!(f, " - object: {}", or_dash(item.object.as_deref()))?;
            if let Some(size) = item.size {
                writeln!(f, " - size: {}", bytes(size))?;
            }
            if let Some(at) = &item.retained_at {
                writeln!(f, " - retained at: {at}")?;
                writeln!(
                    f,
                    " - retained by: {}",
                    or_dash(item.retained_by.as_deref())
                )?;
            }
            writeln!(f, " - meta: {}", or_dash(item.meta.as_deref()))?;

            for binding in &placement.bindings {
                writeln!(f, " - binding {}: {}", binding.source, binding.handle)?;

                if !binding.base {
                    writeln!(f, "    - base: none")?;
                } else {
                    writeln!(
                        f,
                        "    - base object: {}",
                        or_dash(binding.base_object.as_deref())
                    )?;
                    writeln!(
                        f,
                        "    - base flags: {}",
                        or_dash(
                            binding
                                .base_flags
                                .as_ref()
                                .map(|flags| flags.join(" "))
                                .filter(|flags| !flags.is_empty())
                                .as_deref()
                        )
                    )?;
                    writeln!(
                        f,
                        "    - base revision: {}",
                        or_dash(binding.base_revision.as_deref())
                    )?;
                }

                // NOTE: the exception line. Printing it only when it applies
                // is what makes a diverged binding stand out from the
                // ordinary ones beside it.
                if binding.conflicted {
                    writeln!(
                        f,
                        "    - conflicted at revision: {}",
                        or_dash(binding.conflict_revision.as_deref())
                    )?;
                }
            }
        }

        Ok(())
    }
}

/// The `item export --output` output.
#[derive(Debug, Serialize)]
pub struct ItemExportOutput {
    /// The exported item.
    pub seq: i64,
    /// The collection it was read from.
    pub collection: String,
    /// The body's content hash.
    pub hash: String,
    /// How many bytes were written.
    pub bytes: u64,
    /// Where they were written.
    pub path: PathBuf,
}

impl fmt::Display for ItemExportOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Wrote {} of item {} to {}",
            bytes(self.bytes),
            self.seq,
            self.path.display()
        )
    }
}

/// What became of a restore.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestoreStatus {
    /// The action was applied and the item is live again.
    Applied,
    /// Another writer holds the store lock, so the action waits in the queue.
    Queued,
    /// The drain ran and the item is still not live: the action was refused.
    Refused,
}

/// The `item restore` output.
#[derive(Debug, Serialize)]
pub struct ItemRestoreOutput {
    /// The restored item.
    pub seq: i64,
    /// The collection it returns to.
    pub collection: String,
    /// Its cross-source link id.
    pub link_id: String,
    /// The queue row the restore was appended as.
    pub action: i64,
    /// What became of that row.
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
                self.seq, self.collection, self.action
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
#[derive(Debug, Serialize)]
pub struct ItemPurgeOutput {
    /// The cutoff a time-based purge used, `None` for a single item.
    pub cutoff: Option<String>,
    /// How many items were destroyed.
    pub items: usize,
}

impl fmt::Display for ItemPurgeOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Purged {} item(s)", self.items)?;
        if self.items > 0 {
            writeln!(f, "Run `pimdir gc` to reclaim the bodies they released")?;
        }
        Ok(())
    }
}
