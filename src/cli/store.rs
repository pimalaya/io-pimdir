//! The `store` verb group: the one-screen summary of a store.

use std::fmt;

use anyhow::Result;
use clap::{Args, Subcommand};
use pimalaya_cli::{columns::Columns, fields::Fields, printer::Printer, sanitize};
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
        let sync_version = self.sync_version.as_deref().unwrap_or("-");
        let sources = self.sources.join(",");
        let sources = if sources.is_empty() { "-" } else { &sources };
        let mut fields = Fields::default();

        fields.push("Path", sanitize::text(&self.path));
        fields.push("Store id", &self.store_id);
        fields.push("Incarnation", self.incarnation);
        fields.push("Format version", &self.format_version);
        fields.push("Schema version", self.schema_version);
        fields.push("Read version", self.read_version);
        fields.push("Produce version", self.produce_version);
        fields.push("Derivation", self.derivation);
        fields.push("Sync version", sanitize::text(sync_version));
        fields.push("Sources", sanitize::text(sources));
        fields.push("Live items", self.live_items);
        fields.push("Retained items", self.retained_items);
        fields.push("Objects", self.objects);
        fields.push("Object bytes", bytes(self.object_bytes));
        fields.push("Live bytes", bytes(self.live_bytes));
        fields.push("Retained bytes", bytes(self.retained_bytes));
        fields.push("Last change", self.last_change);
        fields.push("Purges", self.purges);

        write!(f, "{fields}")?;

        if self.collections.is_empty() {
            return Ok(());
        }

        // NOTE: the collection id is the free-text column, so it reads last.
        let mut columns = Columns::new(["LIVE", "RETAINED", "COLLECTION"]);

        for collection in &self.collections {
            columns.push([
                collection.live.to_string(),
                collection.retained.to_string(),
                sanitize::text(&collection.id).into_owned(),
            ]);
        }

        write!(f, "\n{columns}")
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::store::*;

    #[test]
    fn the_summary_prints_fields_then_its_collections() {
        let output = StoreInfoOutput {
            path: String::from("/var/pimdir"),
            store_id: String::from("e2646df031cd876334410c4df14ebd86"),
            incarnation: 1,
            format_version: String::from("storage-draft-10"),
            schema_version: 1,
            read_version: 1,
            produce_version: 1,
            derivation: 3,
            sync_version: None,
            sources: vec![String::from("left"), String::from("right")],
            collections: vec![
                StoreCollectionCount {
                    id: String::from("Archives/2026"),
                    live: 120,
                    retained: 0,
                },
                StoreCollectionCount {
                    id: String::from("INBOX"),
                    live: 2,
                    retained: 1,
                },
            ],
            live_items: 122,
            retained_items: 1,
            objects: 123,
            object_bytes: 2048,
            live_bytes: 1536,
            retained_bytes: 512,
            last_change: 11,
            purges: 0,
        };

        let expected = "\
Path:             /var/pimdir
Store id:         e2646df031cd876334410c4df14ebd86
Incarnation:      1
Format version:   storage-draft-10
Schema version:   1
Read version:     1
Produce version:  1
Derivation:       3
Sync version:     -
Sources:          left,right
Live items:       122
Retained items:   1
Objects:          123
Object bytes:     2.0 KiB
Live bytes:       1.5 KiB
Retained bytes:   512 B
Last change:      11
Purges:           0

LIVE  RETAINED  COLLECTION
120   0         Archives/2026
2     1         INBOX
";
        assert_eq!(output.to_string(), expected);
    }
}
