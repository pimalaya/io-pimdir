//! The `store` verb group: the one-screen summary of a store.

use std::fmt;

use anyhow::Result;
use clap::{Args, Subcommand};
use pimalaya_cli::{
    printer::Printer,
    table::{Cell, ContentArrangement, Table, presets::UTF8_FULL},
};
use schemars::JsonSchema;
use serde::Serialize;

use crate::cli::{StoreFlags, bytes, error::report};

/// Inspect the store as a whole.
#[derive(Debug, Subcommand)]
pub enum StoreCommand {
    /// Summarise the store: identity, versions, sources, counts and bytes.
    Info(StoreInfoCommand),
}

impl StoreCommand {
    /// Runs the selected subcommand.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        match self {
            Self::Info(cmd) => cmd.execute(printer, store),
        }
    }
}

/// Summarise the store: identity, versions, sources, counts, bytes and change
/// feed.
///
/// The counts are per collection, live and retained; the retained bytes are
/// what `item purge` would reclaim. The store id, its incarnation and the
/// change cursor are what a consumer deriving from the store (a search index)
/// records.
#[derive(Debug, Args)]
pub struct StoreInfoCommand;

impl StoreInfoCommand {
    /// Prints the summary.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let path = store.dir().display().to_string();
        let read = store.read()?;

        let mut collections = Vec::new();
        let mut live = 0;
        let mut retained = 0;

        for collection in read.list_collections().map_err(report)? {
            let collection_live = read.count_items(&collection.id).map_err(report)?;
            let collection_retained =
                read.count_retained(&collection.id).map_err(report)?.max(0) as u64;
            live += collection_live;
            retained += collection_retained;
            collections.push(StoreCollectionCount {
                id: collection.id,
                live: collection_live,
                retained: collection_retained,
            });
        }

        let objects = read.object_stats().map_err(report)?;
        let live_bytes = read.live_bytes().map_err(report)?;
        let retained_bytes = read.retained_bytes().map_err(report)?;
        let cursor = read.change_cursor().map_err(report)?;
        let meta = read.store_meta().map_err(report)?;

        printer.out(StoreInfoOutput {
            path,
            store_id: meta.store_id,
            incarnation: meta.incarnation,
            format_version: meta.format_version,
            schema_version: meta.version,
            read_version: meta.read_version,
            produce_version: meta.produce_version,
            derivation: meta.derivation,
            sync_version: meta.sync_version,
            sources: read.distinct_sources().map_err(report)?,
            collections,
            live_items: live,
            retained_items: retained,
            objects: objects.count,
            object_bytes: objects.bytes,
            live_bytes,
            retained_bytes,
            last_change: cursor.changed,
            purges: cursor.purges,
        })
    }
}

/// One collection's item counts.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StoreCollectionCount {
    /// The collection id.
    pub id: String,
    /// Live items.
    pub live: u64,
    /// Retained items.
    pub retained: u64,
}

/// The `store info` output.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StoreInfoOutput {
    /// The store directory.
    pub path: String,
    /// The store's id, 32 lowercase hex digits drawn at its creation.
    pub store_id: String,
    /// The store's incarnation, raised when a copy of it is restored.
    pub incarnation: i64,
    /// The STORAGE version whose schema the store holds, the one this build
    /// opens, since the reader refuses any other.
    pub format_version: String,
    /// The schema version.
    pub schema_version: i64,
    /// The lowest schema version a reader must implement to read the store.
    pub read_version: i64,
    /// The lowest schema version a producer must implement to enqueue.
    pub produce_version: i64,
    /// The Annex A revision the store's summaries stand under.
    pub derivation: i64,
    /// The SYNC version whose engine wrote the sync state, none before an
    /// engine wrote.
    pub sync_version: Option<String>,
    /// The sources the store has synced against.
    pub sources: Vec<String>,
    /// Per-collection counts.
    pub collections: Vec<StoreCollectionCount>,
    /// Live items, store-wide.
    pub live_items: u64,
    /// Retained items, store-wide.
    pub retained_items: u64,
    /// Indexed objects.
    pub objects: u64,
    /// What every indexed object weighs.
    pub object_bytes: u64,
    /// What the objects a live item still binds weigh.
    pub live_bytes: u64,
    /// What retention is holding, and what a purge would reclaim.
    pub retained_bytes: u64,
    /// The last change stamp drawn, above which the feed is read.
    pub last_change: i64,
    /// How many rows left the store without a stamp.
    pub purges: i64,
}

impl fmt::Display for StoreInfoOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Store at {}", self.path)?;
        writeln!(
            f,
            " - id: {}, incarnation {}",
            self.store_id, self.incarnation
        )?;
        writeln!(
            f,
            " - format: {}, schema version {} (read {}, produce {})",
            self.format_version, self.schema_version, self.read_version, self.produce_version
        )?;
        writeln!(
            f,
            " - derivation: revision {}, sync: {}",
            self.derivation,
            self.sync_version.as_deref().unwrap_or("none yet")
        )?;
        writeln!(
            f,
            " - sources: {}",
            if self.sources.is_empty() {
                String::from("none yet")
            } else {
                self.sources.join(", ")
            }
        )?;
        writeln!(
            f,
            " - items: {} live, {} retained",
            self.live_items, self.retained_items
        )?;
        writeln!(
            f,
            " - objects: {} ({} total, {} live, {} retained)",
            self.objects,
            bytes(self.object_bytes),
            bytes(self.live_bytes),
            bytes(self.retained_bytes)
        )?;
        writeln!(
            f,
            " - change feed: last stamp {}, {} purge(s)",
            self.last_change, self.purges
        )?;

        if self.collections.is_empty() {
            return writeln!(f, " - collections: none yet");
        }

        writeln!(f)?;

        let mut table = Table::new();
        table
            .load_style(UTF8_FULL)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .set_header(vec![
                Cell::new("COLLECTION"),
                Cell::new("LIVE"),
                Cell::new("RETAINED"),
            ]);

        for collection in &self.collections {
            table.add_row(vec![
                Cell::new(&collection.id),
                Cell::new(collection.live),
                Cell::new(collection.retained),
            ]);
        }

        writeln!(f, "{table}")
    }
}
