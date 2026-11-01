//! The `quarantine` verb group: the bodies that failed their name.
//!
//! The owner moves a body whose bytes do not hash to its name into
//! quarantine/ rather than unlinking it (spec §7), since it may be the
//! only copy of what it once was, and nothing reads it there again. Only
//! the user empties it, through this group; `pimdir check` lists it.

use std::fmt;

use anyhow::Result;
use clap::{Args, Subcommand};
use pimalaya_cli::printer::Printer;
use schemars::JsonSchema;
use serde::Serialize;

use crate::cli::{StoreFlags, bytes, confirm, report};

/// Manage the bodies that failed their name.
#[derive(Debug, Subcommand)]
pub enum QuarantineCommand {
    /// Unlink every file of quarantine/.
    Empty(QuarantineEmptyCommand),
}

impl QuarantineCommand {
    /// Runs the selected subcommand.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        match self {
            Self::Empty(cmd) => cmd.execute(printer, store),
        }
    }
}

/// Unlink every file of quarantine/.
///
/// A quarantined file is a body that failed its name, kept in case it was
/// the only copy of something; `pimdir check` lists them. Emptying cannot be
/// undone. Takes the owner role, fails while a producer is staging a body,
/// and asks first unless `--yes` is passed.
#[derive(Debug, Args)]
pub struct QuarantineEmptyCommand {
    /// Do not ask for confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

impl QuarantineEmptyCommand {
    /// Empties quarantine/.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let owner = store.owner()?;
        let quarantined = owner.blobs().quarantined()?.len();
        if quarantined > 0 {
            confirm(
                printer,
                self.yes,
                &format!("Unlink the {quarantined} quarantined file(s) for good?"),
            )?;
        }

        let emptied = owner.empty_quarantine().map_err(report)?;

        printer.out(QuarantineEmptyOutput {
            files: emptied.files,
            bytes: emptied.bytes,
            left: emptied.left.len(),
        })
    }
}

/// The `quarantine empty` output.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QuarantineEmptyOutput {
    /// Files and links unlinked.
    pub files: usize,
    /// Bytes they held.
    pub bytes: u64,
    /// Entries left in place, neither a file nor a link.
    pub left: usize,
}

impl fmt::Display for QuarantineEmptyOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.files {
            0 => writeln!(f, "Nothing in quarantine")?,
            files => writeln!(
                f,
                "Unlinked {files} quarantined file(s), freeing {}",
                bytes(self.bytes)
            )?,
        }
        if self.left > 0 {
            writeln!(
                f,
                "Left {} entry(ies) in quarantine/ that are no file",
                self.left
            )?;
        }
        Ok(())
    }
}
