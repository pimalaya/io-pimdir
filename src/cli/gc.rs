//! The `gc` verb: the collector.
//!
//! A store never collects itself. Writes maintain the refcounts and stop
//! there, so an object nothing references is unreferenced rather than
//! deleted, and a body a consumer streamed into the blob tree survives
//! until the batch that attaches it, which is what spec §14 invites. What
//! reclaims is this verb, when an operator or a schedule asks.

use std::fmt;

use anyhow::Result;
use clap::Args;
use pimalaya_cli::printer::Printer;
use schemars::JsonSchema;
use serde::Serialize;

use crate::cli::{StoreFlags, bytes, error::report};

/// Reclaim what nothing references any more.
///
/// Drops the object rows at refcount zero, unlinks their bodies, and sweeps
/// the orphan blob files and the temporary files a crash can leave. Takes the
/// owner role, so it never runs beside a sync, and fails while a producer is
/// mid-append. Nothing a live or retained item still points at is touched.
#[derive(Debug, Args)]
pub struct GcCommand;

impl GcCommand {
    /// Runs the collector.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let collected = store.owner()?.collect_garbage().map_err(report)?;

        printer.out(GcOutput {
            objects: collected.objects,
            blobs: collected.blobs,
            bytes: collected.bytes,
            temporaries: collected.temporaries,
            strays: collected.strays.len(),
        })
    }
}

/// The `gc` output.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GcOutput {
    /// Object rows dropped.
    pub objects: usize,
    /// Blob files unlinked.
    pub blobs: usize,
    /// Bytes the unlinked files freed.
    pub bytes: u64,
    /// Temporary files unlinked: what a writer killed before its own
    /// unlink left under objects/.
    pub temporaries: usize,
    /// Entries of objects/ that are no body, left in place: `pimdir check`
    /// lists them.
    pub strays: usize,
}

impl fmt::Display for GcOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.objects == 0 && self.blobs == 0 && self.temporaries == 0 {
            writeln!(f, "Nothing to collect: every body is still referenced")?;
        } else if self.objects == 0 && self.blobs == 0 {
            writeln!(
                f,
                "Collected {} temporary file(s), freeing {}",
                self.temporaries,
                bytes(self.bytes)
            )?;
        } else if self.temporaries > 0 {
            writeln!(
                f,
                "Collected {} object(s), {} blob file(s) and {} temporary file(s), freeing {}",
                self.objects,
                self.blobs,
                self.temporaries,
                bytes(self.bytes)
            )?;
        } else {
            writeln!(
                f,
                "Collected {} object(s) and {} blob file(s), freeing {}",
                self.objects,
                self.blobs,
                bytes(self.bytes)
            )?;
        }

        if self.strays > 0 {
            writeln!(
                f,
                "Left {} entry(ies) under objects/ that are no body, see `pimdir check`",
                self.strays
            )?;
        }
        Ok(())
    }
}
