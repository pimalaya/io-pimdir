//! The `collection` verb group: what a store holds, one row per collection.

use std::fmt;

use anyhow::Result;
use clap::{Args, Subcommand};
use pimalaya_cli::{
    printer::Printer,
    table::{Cell, ContentArrangement, Table, presets::UTF8_FULL},
};
use schemars::JsonSchema;
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

/// List every collection with its counts.
///
/// One row per collection: its id, declared media type, display name,
/// handle-space generation, coverage, live and retained item counts. The
/// coverage is the scope every source of the collection has listed whole
/// (`all`, or since a date and until one), `-` while one of them has never
/// closed a round: below it, the store may lack mail the server holds. The
/// retained count is what a delete left behind: hidden from
/// every read and from the sync, and only `item purge` destroys them.
#[derive(Debug, Args)]
pub struct CollectionListCommand;

impl CollectionListCommand {
    /// Lists the collections and their counts.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let store = store.read()?;
        let mut rows = Vec::new();

        for collection in store.list_collections().map_err(report)? {
            rows.push(CollectionRow {
                live: store.count_items(&collection.id).map_err(report)?,
                retained: store.count_retained(&collection.id).map_err(report)?.max(0) as u64,
                id: collection.id,
                kind: collection.kind,
                name: collection.name,
                generation: collection.generation,
                role: collection.role,
                covered_since: collection
                    .coverage
                    .as_ref()
                    .and_then(|c| c.scope.since.clone()),
                covered_until: collection
                    .coverage
                    .as_ref()
                    .and_then(|c| c.scope.until.clone()),
                covered_at: collection.coverage.map(|c| c.at),
            });
        }

        printer.out(CollectionsOutput(rows))
    }
}

/// One collection as the listing shows it.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
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
    /// What the source states the collection is for (`sent`, `default`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Live items.
    pub live: u64,
    /// The floor of the scope every source has listed whole, `None` for
    /// none or while one source never closed a round.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub covered_since: Option<String>,
    /// Its ceiling, exclusive, `None` for none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub covered_until: Option<String>,
    /// When the oldest of those rounds closed, `None` while one source
    /// never closed one: no coverage at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub covered_at: Option<String>,
    /// Retained (soft-deleted) items.
    pub retained: u64,
}

impl CollectionRow {
    /// The coverage as the table shows it.
    fn coverage(&self) -> String {
        if self.covered_at.is_none() {
            return String::from("-");
        }
        match (&self.covered_since, &self.covered_until) {
            (None, None) => String::from("all"),
            (Some(since), None) => format!("since {since}"),
            (None, Some(until)) => format!("until {until}"),
            (Some(since), Some(until)) => format!("{since} to {until}"),
        }
    }
}

/// The `collection list` output.
#[derive(Debug, Serialize, JsonSchema)]
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
                Cell::new("ROLE"),
                Cell::new("GEN"),
                Cell::new("LIVE"),
                Cell::new("COVERED"),
                Cell::new("RETAINED"),
            ]);

        for row in &self.0 {
            table.add_row(vec![
                Cell::new(&row.id),
                Cell::new(or_dash(Some(row.kind.as_str()).filter(|k| !k.is_empty()))),
                Cell::new(&row.name),
                Cell::new(or_dash(row.role.as_deref())),
                Cell::new(row.generation),
                Cell::new(row.live),
                Cell::new(row.coverage()),
                Cell::new(row.retained),
            ]);
        }

        writeln!(f, "{table}")
    }
}
