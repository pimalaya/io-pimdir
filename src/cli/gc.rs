//! The `gc` verb: the collector.
//!
//! A store never collects itself. Writes maintain the refcounts and stop there,
//! so an object nothing references is unreferenced rather than deleted, and a
//! body a consumer streamed into the blob tree survives until the batch that
//! attaches it — which is what spec §14 invites and a sweep at the end of every
//! write silently broke. What reclaims is this verb, when an operator or a
//! schedule asks for it.

use std::fmt;

use anyhow::Result;
use clap::Args;
use pimalaya_cli::printer::Printer;
use serde::Serialize;

use crate::cli::{StoreFlags, bytes, report};

/// Reclaim what nothing references any more.
///
/// Drops the object rows at refcount zero, unlinks their bodies, and sweeps
/// the orphan blob files a crash can leave. Takes the owner role, so it never
/// runs beside a sync, and waits for any producer that is mid-append. Nothing
/// a live or retained item still points at is touched.
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
        })
    }
}

/// The `gc` output.
#[derive(Debug, Serialize)]
pub struct GcOutput {
    /// Object rows dropped.
    pub objects: usize,
    /// Blob files unlinked.
    pub blobs: usize,
    /// Bytes they freed.
    pub bytes: u64,
}

impl fmt::Display for GcOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.objects == 0 && self.blobs == 0 {
            return writeln!(f, "Nothing to collect: every body is still referenced");
        }

        writeln!(
            f,
            "Collected {} object(s) and {} blob file(s), freeing {}",
            self.objects,
            self.blobs,
            bytes(self.bytes)
        )
    }
}
