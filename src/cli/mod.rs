//! The `pimdir` command tree, one module per verb group.
//!
//! Each module owns one group of subcommands ([`collection`], [`item`],
//! [`queue`], [`store`], [`check`], [`export`]) and renders its own output as
//! both text and JSON. This module holds what they share: the global store
//! flags, the roles a verb may open the store with, the human-facing
//! error mapping, the confirmation prompt guarding destructive verbs and the
//! small formatting helpers.
//!
//! The modules hang off the binary crate root rather than off the library's
//! `lib.rs`: the command structs are the binary's business, so the library's
//! public API stays free of clap.

pub mod check;
pub mod collection;
pub mod db;
pub mod export;
pub mod gc;
pub mod item;
pub mod queue;
pub mod store;

use std::{
    io::{IsTerminal, stdin, stdout},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use anyhow::{Result, anyhow, bail};
use clap::Args;
use io_pimdir::{PimdirBlobs, PimdirError, PimdirProducer, PimdirStore};
use pimalaya_cli::{clap::parsers::path_parser, printer::Printer, prompt};

use crate::cli::db::PimdirDb;

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
    /// `phone`, …). Only a verb that writes as a side reads this flag, which
    /// today is `item restore`: it says which source the restored item is
    /// created for. A single-source store needs no flag.
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
            bail!(
                "no pimdir store at {}: {} not found",
                self.store.display(),
                db.display()
            );
        }
        Ok(())
    }

    /// Opens the store read-only: the role every inspection verb uses, safe to
    /// run while a sync holds the write lock.
    pub fn read(&self) -> Result<PimdirStore> {
        self.ensure_store()?;
        PimdirStore::open_read_only(&self.store).map_err(report)
    }

    /// Opens the store as its owner, for a verb whose effect is not tied to one
    /// source (purge, queue cancellation, orphan sweep).
    ///
    /// Fails when another process owns the store (spec §8): a verb that
    /// destroys or repairs has nothing useful to do without the role.
    pub fn owner(&self) -> Result<PimdirStore> {
        self.ensure_store()?;
        PimdirStore::open(&self.store).map_err(report)
    }

    /// The owner role when it is free, `None` when another process holds it:
    /// what a verb that can leave its work queued reports instead of failing.
    pub fn owner_if_free(&self) -> Result<Option<PimdirStore>> {
        self.ensure_store()?;
        match PimdirStore::open(&self.store) {
            Ok(store) => Ok(Some(store)),
            Err(PimdirError::Owned(_)) => Ok(None),
            Err(err) => Err(report(err)),
        }
    }

    /// The source a queued mutation is staged for: the flag when given, else
    /// the store's own when it has exactly one.
    ///
    /// A store syncing several sources without `--source` is refused rather
    /// than guessed: creating an item for the wrong side would push it to the
    /// wrong server. A store that has synced no source at all is refused too,
    /// since there is no side to act as.
    pub fn write_source(&self) -> Result<String> {
        if let Some(source) = &self.source {
            return Ok(source.clone());
        }

        let sources = self.read()?.distinct_sources().map_err(report)?;
        match sources.len() {
            1 => Ok(sources.into_iter().next().unwrap()),
            0 => bail!("this store syncs no source yet: name the one to write as with --source"),
            _ => bail!(
                "this store syncs several sources ({}): pick the one to write as with --source",
                sources.join(", ")
            ),
        }
    }

    /// Opens the store as a producer: the enqueue-only role, which never needs
    /// the owner's write batches and coexists with a running sync.
    pub fn producer(&self) -> Result<PimdirProducer> {
        self.ensure_store()?;
        PimdirProducer::open(&self.store, PRODUCER).map_err(report)
    }

    /// The blob directory handle, for reading a body back, bound to the hash
    /// the store names its bodies by.
    pub fn blobs(&self) -> Result<PimdirBlobs> {
        let store = PimdirStore::open_read_only(&self.store).map_err(report)?;
        Ok(store.blobs())
    }

    /// The read-only diagnostic connection (see [`db`]).
    pub fn db(&self) -> Result<PimdirDb> {
        self.ensure_store()?;
        PimdirDb::open(&self.store)
    }
}

/// Turns a store error into an operator-facing one.
///
/// The lock contention case is the one that matters: it is not a failure of the
/// command, it means somebody else is writing, so it reports as a sentence
/// naming the likely cause instead of a raw error dump.
pub fn report(err: PimdirError) -> anyhow::Error {
    match err {
        PimdirError::Owned(store) => {
            anyhow!(
                "another process owns the store at {} (a sync is running?); retry once it releases",
                store.display()
            )
        }
        PimdirError::Staging(store) => {
            anyhow!(
                "a producer is staging a body in the store at {} (a frontend is open?); \
                 retry once it is done",
                store.display()
            )
        }
        PimdirError::Busy => {
            anyhow!(
                "another writer holds the store lock (a sync is running?); retry once it releases"
            )
        }
        err => anyhow!(err),
    }
}

/// Asks before destroying data, unless `--yes` was passed.
///
/// Refuses rather than prompts when the output is JSON or not a terminal: a
/// confirmation written into a pipe is not a confirmation.
pub fn confirm(printer: &impl Printer, yes: bool, question: &str) -> Result<()> {
    if yes {
        return Ok(());
    }

    if printer.is_json() || !stdout().is_terminal() || !stdin().is_terminal() {
        bail!("refusing to destroy data without a confirmation: pass --yes to proceed");
    }

    if !prompt::bool(question, false)? {
        bail!("cancelled");
    }

    Ok(())
}

/// The current instant as the RFC 3339 stamp the store's own timestamps use
/// (UTC, milliseconds), so a stamp this tool writes sorts against them.
pub fn now() -> String {
    humantime::format_rfc3339_millis(SystemTime::now()).to_string()
}

/// The RFC 3339 instant `age` ago, the cutoff a time-based purge passes to the
/// store. An age reaching before the epoch clamps to the epoch.
pub fn cutoff(age: Duration) -> String {
    let cutoff = SystemTime::now()
        .checked_sub(age)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    humantime::format_rfc3339_millis(cutoff).to_string()
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

/// A value that may be absent, rendered as a dash in a table cell.
pub fn or_dash(value: Option<&str>) -> &str {
    value.unwrap_or("-")
}
