//! The `store` verb group: the one-screen summary of a store.

use std::fmt;

use anyhow::Result;
use clap::{Args, Subcommand};
use io_replica::collection::ReplicaCollectionId;
use pimalaya_cli::{
    printer::Printer,
    table::{Cell, ContentArrangement, Table, presets::UTF8_FULL},
};
use serde::Serialize;

use crate::cli::{StoreFlags, bytes, report};

/// Inspect the store as a whole.
#[derive(Debug, Subcommand)]
pub enum StoreCommand {
    /// Summarise the store: schema, sources, counts and bytes.
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

/// Summarise the store: its schema version, the sources it syncs, its
/// per-collection live and retained counts, and how many bytes its bodies
/// weigh, split between the live ones and the ones retention is holding.
///
/// The retained bytes are what `item purge` would reclaim.
#[derive(Debug, Args)]
pub struct StoreInfoCommand;

impl StoreInfoCommand {
    /// Prints the summary.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let path = store.dir().display().to_string();
        let db = store.db()?;
        let read = store.read()?;

        let mut collections = Vec::new();
        let mut live = 0;
        let mut retained = 0;

        for collection in read.list_collections().map_err(report)? {
            let id = ReplicaCollectionId(collection.id.clone());
            let collection_live = read.count_items(&collection.id).map_err(report)?;
            let collection_retained = read.count_retained(&id).map_err(report)?.max(0) as u64;
            live += collection_live;
            retained += collection_retained;
            collections.push(StoreCollectionCount {
                id: collection.id,
                live: collection_live,
                retained: collection_retained,
            });
        }

        let objects = db.object_stats()?;
        let live_bytes = db.live_bytes()?;
        let retained_bytes = read.retained_bytes().map_err(report)?;

        printer.out(StoreInfoOutput {
            path,
            schema_version: db.version()?,
            supported_schema_version: io_pimdir::sql::VERSION,
            sources: read.distinct_sources().map_err(report)?,
            collections,
            live_items: live,
            retained_items: retained,
            objects: objects.count,
            object_bytes: objects.bytes,
            live_bytes,
            retained_bytes,
        })
    }
}

/// One collection's item counts.
#[derive(Debug, Serialize)]
pub struct StoreCollectionCount {
    /// The collection id.
    pub id: String,
    /// Live items.
    pub live: u64,
    /// Retained items.
    pub retained: u64,
}

/// The `store info` output.
#[derive(Debug, Serialize)]
pub struct StoreInfoOutput {
    /// The store directory.
    pub path: String,
    /// The schema version the store is stamped with.
    pub schema_version: i64,
    /// The schema version this build services.
    pub supported_schema_version: i64,
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
}

impl fmt::Display for StoreInfoOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Store at {}", self.path)?;
        writeln!(
            f,
            " - schema version: {} (this build services {})",
            self.schema_version, self.supported_schema_version
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
