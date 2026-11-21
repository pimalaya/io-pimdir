//! The `pimdir` command tree, one module per verb group.
//!
//! Each module owns one group of subcommands ([`collection`], [`item`],
//! [`property`], [`queue`], [`store`], [`check`], [`gc`], [`quarantine`],
//! [`export`], [`search`], [`version`]) and renders its own output as both
//! text and JSON, with [`json_schema`] indexing every output type. This
//! module holds what they share: the global store flags, the roles a verb
//! may open the store with, the error mapping, the confirmation prompt
//! guarding destructive verbs and the formatting helpers.
//!
//! The modules hang off the binary crate root rather than the library's
//! `lib.rs`: the command structs are the binary's business, so the
//! library's public API stays free of clap.

pub mod check;
pub mod collection;
pub mod error;
pub mod export;
pub mod gc;
pub mod item;
pub mod json_schema;
pub mod property;
pub mod quarantine;
pub mod queue;
pub mod search;
pub mod store;
pub mod summary;
pub mod version;

use std::{
    io::{IsTerminal, stdin, stdout},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::Result;
use clap::Args;
use io_pimdir::client::{PimdirError, PimdirStore, producer::PimdirProducer, reader::PimdirReader};
use pimalaya_cli::{clap::parsers::path_parser, printer::Printer, prompt, sanitize};

use crate::cli::error::{ErrorCode, report};

/// The producer name recorded on every queue row this tool appends, so an
/// operator reading `queue list` can tell CLI-originated actions apart from a
/// frontend's.
pub const PRODUCER: &str = "pimdir-cli";

/// Where the store is and which source to act as, shared by every command.
#[derive(Debug, Args)]
pub struct StoreFlags {
    /// Path to the pimdir store directory.
    ///
    /// The directory holding `pimdir.db` and the `objects/` blob tree.
    /// Defaults to the current directory.
    #[arg(long, short = 's', global = true)]
    #[arg(value_name = "PATH", value_parser = path_parser, default_value = ".")]
    pub store: PathBuf,

    /// Act as this source rather than the store's own.
    ///
    /// A store records one source per side it syncs with (`left`, `right`,
    /// `phone`, …). Only the verbs that write as a side read this flag:
    /// `item restore` creates the restored item for it, and `tag` stages
    /// only the placements of collections it syncs, picking it where several
    /// sources sync one. A single-source store needs no flag.
    #[arg(long, global = true, value_name = "NAME")]
    pub source: Option<String>,
}

impl StoreFlags {
    /// The store directory.
    pub fn dir(&self) -> &Path {
        &self.store
    }

    /// Fails unless the directory really holds a store, so a mistyped path
    /// reports as a missing store instead of silently creating an empty one.
    pub fn ensure_store(&self) -> Result<()> {
        let db = self.store.join("pimdir.db");
        if !db.is_file() {
            return Err(ErrorCode::StoreUncreated.error(format!(
                "no pimdir store at {}: {} not found",
                self.store.display(),
                db.display()
            )));
        }
        Ok(())
    }

    /// Opens the store to read: the role every inspection verb uses, safe to
    /// run while a sync holds the write lock.
    ///
    /// Without the pending overlay (spec §15.4): an operator inspects the
    /// store as it stands, and reads the queue through `queue list`.
    pub fn read(&self) -> Result<PimdirReader> {
        self.ensure_store()?;
        PimdirReader::open(&self.store).map_err(report)
    }

    /// Opens the store as its owner, for a verb whose effect is not tied to one
    /// source (purge, queue cancellation, orphan sweep).
    ///
    /// Fails when another process owns the store (spec §8): a verb that
    /// destroys or repairs has nothing useful to do without the role. Opened
    /// as the store stands (`open_current`): this tool is never the store's
    /// sync engine, so it never creates one.
    pub fn owner(&self) -> Result<PimdirStore> {
        self.ensure_store()?;
        PimdirStore::open_current(&self.store).map_err(report)
    }

    /// The owner role when it is free, `None` when another process holds it:
    /// what a verb that can leave its work queued reports instead of failing.
    /// Opened as the store stands, as [`owner`](Self::owner): a store holding
    /// no schema yet is `None` too, its work left queued for the sync engine
    /// that creates it.
    pub fn owner_if_free(&self) -> Result<Option<PimdirStore>> {
        self.ensure_store()?;
        match PimdirStore::open_current(&self.store) {
            Ok(store) => Ok(Some(store)),
            Err(PimdirError::Owned(_) | PimdirError::NotCurrent(_)) => Ok(None),
            Err(err) => Err(report(err)),
        }
    }

    /// The source a queued mutation is staged for: the flag when given, else
    /// the store's own when it has exactly one, read on the handle the verb
    /// already holds.
    ///
    /// A store syncing several sources without `--source` is refused rather
    /// than guessed: creating an item for the wrong side would push it to the
    /// wrong server. A store that has synced no source at all is refused too,
    /// since there is no side to act as.
    pub fn write_source(&self, read: &PimdirReader) -> Result<String> {
        if let Some(source) = &self.source {
            return Ok(source.clone());
        }

        let sources = read.distinct_sources().map_err(report)?;
        match sources.len() {
            1 => Ok(sources.into_iter().next().unwrap()),
            0 => Err(ErrorCode::SourceRequired
                .error("this store syncs no source yet: name the one to write as with --source")),
            _ => Err(ErrorCode::SourceRequired.error(format!(
                "this store syncs several sources ({}): pick the one to write as with --source",
                sources.join(", ")
            ))),
        }
    }

    /// Opens the store as a producer: the enqueue-only role, which never needs
    /// the owner's write batches and coexists with a running sync.
    pub fn producer(&self) -> Result<PimdirProducer> {
        self.ensure_store()?;
        PimdirProducer::open(&self.store, PRODUCER).map_err(report)
    }
}

/// Asks before destroying data, or doing what cannot be undone, unless
/// `--yes` was passed.
///
/// Refuses rather than prompts when the output is JSON or not a terminal: a
/// confirmation written into a pipe is not a confirmation.
pub fn confirm(printer: &impl Printer, yes: bool, question: &str) -> Result<()> {
    if yes {
        return Ok(());
    }

    if printer.is_json() || !stdout().is_terminal() || !stdin().is_terminal() {
        return Err(ErrorCode::ConfirmationRequired
            .error("refusing to proceed without a confirmation: pass --yes to proceed"));
    }

    if !prompt::bool(sanitize::text(question), false)? {
        return Err(ErrorCode::Cancelled.error("cancelled"));
    }

    Ok(())
}

/// The current instant as the RFC 3339 stamp the store's own timestamps use
/// (UTC, milliseconds), so a stamp this tool writes sorts against them.
pub fn now() -> String {
    stamp(jiff::Timestamp::now())
}

/// The RFC 3339 instant `age` ago, the cutoff a time-based purge passes to the
/// store. An age reaching before the epoch clamps to the epoch.
pub fn cutoff(age: Duration) -> String {
    let now = jiff::Timestamp::now();
    let cutoff = jiff::SignedDuration::try_from(age)
        .ok()
        .and_then(|age| now.checked_sub(age).ok())
        .filter(|cutoff| *cutoff >= jiff::Timestamp::UNIX_EPOCH)
        .unwrap_or(jiff::Timestamp::UNIX_EPOCH);
    stamp(cutoff)
}

/// An instant as RFC 3339 in UTC with milliseconds.
fn stamp(instant: jiff::Timestamp) -> String {
    instant.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// Parses a duration flag (`2s`, `12h`, `90d`, `2 weeks`), a day being 24
/// hours; a negative one is refused.
pub fn duration(value: &str) -> Result<Duration, String> {
    let span: jiff::Span = value.parse().map_err(|err| format!("{err}"))?;
    let signed = span
        .to_duration(jiff::SpanRelativeTo::days_are_24_hours())
        .map_err(|err| format!("{err}"))?;
    Duration::try_from(signed).map_err(|_| format!("{value} is a negative duration"))
}

/// A byte count in the largest unit that keeps it readable.
pub fn bytes(count: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = count as f64;
    let mut unit = 0;

    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{count} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// A value that may be absent, rendered as a dash in the text output.
pub fn or_dash(value: Option<&str>) -> &str {
    value.unwrap_or("-")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    /// The duration flags read what humantime read for them, a day being 24
    /// hours, and refuse a negative one.
    #[test]
    fn a_duration_flag_reads_the_friendly_forms() {
        let parsed = |value: &str| super::duration(value).unwrap();
        assert_eq!(parsed("2s"), Duration::from_secs(2));
        assert_eq!(parsed("12h"), Duration::from_secs(12 * 3600));
        assert_eq!(parsed("90d"), Duration::from_secs(90 * 86_400));
        assert_eq!(parsed("2weeks"), Duration::from_secs(14 * 86_400));
        assert_eq!(parsed("0s"), Duration::ZERO);
        assert!(super::duration("-1s").is_err());
        assert!(super::duration("soon").is_err());
    }

    /// A stamp sorts against the store's own: RFC 3339, UTC, milliseconds.
    #[test]
    fn a_stamp_is_rfc_3339_in_utc_with_milliseconds() {
        let stamp = super::cutoff(Duration::from_secs(u64::MAX));
        assert_eq!(stamp, "1970-01-01T00:00:00.000Z");
        assert_eq!(super::now().len(), stamp.len());
    }
}
