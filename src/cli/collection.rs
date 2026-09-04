//! The `collection` verb group: what a store holds, one row per collection.

use std::fmt;

use anyhow::Result;
use clap::{Args, Subcommand};
use io_pimdir::collection::PimdirCollectionId;
use pimalaya_cli::{
    printer::Printer,
    table::{Cell, ContentArrangement, Table, presets::UTF8_FULL},
};
use serde::Serialize;

use crate::cli::{StoreFlags, or_dash, report};

/// Inspect the store's collections.
///
/// A collection is a mailbox, an address book or a calendar: the store is
/// kind-agnostic and only records the media type each one declares.
#[derive(Debug, Subcommand)]
pub enum CollectionCommand {
    /// List every collection with its counts.
    List(CollectionListCommand),
}

impl CollectionCommand {
    /// Runs the selected subcommand.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        match self {
            Self::List(cmd) => cmd.execute(printer, store),
        }
    }
}

/// List every collection: its id, declared media type, display name,
/// handle-space generation, live, probed and retained item counts.
///
/// A probe is a handle a source enumerated whose identity is not read yet, so
/// no listing can show it. The retained count is what a delete left behind:
/// hidden from every read and from the sync, and only `item purge` destroys
/// them.
#[derive(Debug, Args)]
pub struct CollectionListCommand;

impl CollectionListCommand {
    /// Lists the collections and their counts.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let store = store.read()?;
        let mut rows = Vec::new();

        for collection in store.list_collections().map_err(report)? {
            let id = PimdirCollectionId(collection.id.clone());
            rows.push(CollectionRow {
                live: store.count_items(&collection.id).map_err(report)?,
                probes: store.count_probes(&collection.id).map_err(report)?,
                retained: store.count_retained(&id).map_err(report)?.max(0) as u64,
                id: collection.id,
                kind: collection.kind,
                name: collection.name,
                generation: collection.generation,
            });
        }

        printer.out(CollectionsOutput(rows))
    }
}

/// One collection as the listing shows it.
#[derive(Debug, Serialize)]
pub struct CollectionRow {
    /// The stable collection id.
    pub id: String,
    /// The declared IANA media type, empty when a sync created it before any
    /// consumer declared one.
    pub kind: String,
    /// The display name.
    pub name: String,
    /// The handle-space epoch.
    pub generation: i64,
    /// Live items.
    pub live: u64,
    /// Handles enumerated but not yet identified.
    pub probes: u64,
    /// Retained (soft-deleted) items.
    pub retained: u64,
}

/// The `collection list` output.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct CollectionsOutput(pub Vec<CollectionRow>);

impl fmt::Display for CollectionsOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return writeln!(f, "This store holds no collection yet");
        }

        let mut table = Table::new();
        table
            .load_style(UTF8_FULL)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .set_header(vec![
                Cell::new("ID"),
                Cell::new("KIND"),
                Cell::new("NAME"),
                Cell::new("GEN"),
                Cell::new("LIVE"),
                Cell::new("PROBED"),
                Cell::new("RETAINED"),
            ]);

        for row in &self.0 {
            table.add_row(vec![
                Cell::new(&row.id),
                Cell::new(or_dash(Some(row.kind.as_str()).filter(|k| !k.is_empty()))),
                Cell::new(&row.name),
                Cell::new(row.generation),
                Cell::new(row.live),
                Cell::new(row.probes),
                Cell::new(row.retained),
            ]);
        }

        writeln!(f, "{table}")
    }
}
