//! The `collection` verb group: what a store holds, one row per collection.

use std::fmt;

use anyhow::Result;
use clap::{Args, Subcommand};
use pimalaya_cli::{columns::Columns, printer::Printer, sanitize};
use schemars::JsonSchema;
use serde::Serialize;

use crate::cli::{StoreFlags, error::report};

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
                kind: Some(collection.kind).filter(|kind| !kind.is_empty()),
                name: collection.name,
                description: collection.description,
                color: collection.color,
                default: collection.role.as_deref() == Some("default"),
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

        printer.out(CollectionsOutput { collections: rows })
    }
}

/// One collection as the listing shows it.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CollectionRow {
    /// The stable collection id.
    pub id: String,
    /// The declared IANA media type, `None` when a sync created it before
    /// any consumer declared one.
    pub kind: Option<String>,
    /// The display name.
    pub name: String,
    /// The description its source states, if any.
    pub description: Option<String>,
    /// The color its source states, if any.
    pub color: Option<String>,
    /// Whether its source states it is the default calendar or address
    /// book (the `default` role, STORAGE §14).
    pub default: bool,
    /// The handle-space epoch.
    pub generation: i64,
    /// What the source states the collection is for (`sent`, `default`).
    pub role: Option<String>,
    /// Live items.
    pub live: u64,
    /// The floor of the scope every source has listed whole, `None` for
    /// none or while one source never closed a round.
    pub covered_since: Option<String>,
    /// Its ceiling, exclusive, `None` for none.
    pub covered_until: Option<String>,
    /// When the oldest of those rounds closed, `None` while one source
    /// never closed one: no coverage at all.
    pub covered_at: Option<String>,
    /// Retained (soft-deleted) items.
    pub retained: u64,
}

impl CollectionRow {
    /// The coverage as the text list shows it, empty for none.
    fn coverage(&self) -> String {
        if self.covered_at.is_none() {
            return String::new();
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
#[serde(rename_all = "camelCase")]
pub struct CollectionsOutput {
    /// The collections.
    pub collections: Vec<CollectionRow>,
}

impl fmt::Display for CollectionsOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.collections.is_empty() {
            return writeln!(f, "This store holds no collection yet");
        }

        // NOTE: the name is the free-text column, so it reads last.
        let header = [
            "ID", "KIND", "ROLE", "GEN", "LIVE", "COVERED", "RETAINED", "NAME",
        ];
        let mut columns = Columns::new(header);

        for row in &self.collections {
            let kind = row.kind.as_deref().unwrap_or_default();
            let role = row.role.as_deref().unwrap_or_default();

            columns.push([
                sanitize::text(&row.id).into_owned(),
                sanitize::text(kind).into_owned(),
                sanitize::text(role).into_owned(),
                row.generation.to_string(),
                row.live.to_string(),
                sanitize::text(&row.coverage()).into_owned(),
                row.retained.to_string(),
                sanitize::text(&row.name).into_owned(),
            ]);
        }

        write!(f, "{columns}")
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::collection::*;

    /// A row with the given id, name and role, no kind and no coverage.
    fn row(id: &str, name: &str, role: Option<&str>) -> CollectionRow {
        CollectionRow {
            id: id.into(),
            kind: None,
            name: name.into(),
            description: None,
            color: None,
            default: false,
            generation: 1,
            role: role.map(Into::into),
            live: 2,
            covered_since: None,
            covered_until: None,
            covered_at: None,
            retained: 0,
        }
    }

    #[test]
    fn the_list_prints_plain_columns_ending_with_the_name() {
        let mut covered = row("INBOX", "Inbox \u{202e}gpj.exe", Some("inbox"));
        covered.kind = Some("message/rfc822".into());
        covered.covered_since = Some("2026-01-01".into());
        covered.covered_at = Some("2026-11-11T10:00:00Z".into());

        let output = CollectionsOutput {
            collections: vec![covered, row("contacts", "Personal contacts", None)],
        };

        let expected = "\
ID        KIND            ROLE   GEN  LIVE  COVERED           RETAINED  NAME
INBOX     message/rfc822  inbox  1    2     since 2026-01-01  0         Inbox \u{fffd}gpj.exe
contacts  -               -      1    2     -                 0         Personal contacts
";
        assert_eq!(output.to_string(), expected);
    }
}
