//! The `check` verb: diagnosis, and the repairs that need no guessing.
//!
//! It reports what should not happen. A refcount is maintained incrementally,
//! so a bug here or a foreign writer could leave **drift** between a count and
//! the references that justify it. Foreign keys are enforced only when the
//! writer enabled them, so a **dangling** row is conceivable. An object row
//! whose blob is **missing** is a read that will fail. An **orphan blob** is
//! the benign one, a file no row references, and `pimdir gc` is what takes it:
//! reclamation is the collector's, and this verb reclaims nothing.
//!
//! `--fix` repairs, which is a different thing from reclaiming: it recomputes
//! the drifted refcounts from the pointers that justify them, and clears the
//! bindings whose item is gone. Both are recoveries of a fact the store already
//! holds. Nothing else is touched, because a wrong repair is worse than a
//! reported inconsistency.

use std::{collections::BTreeSet, fmt, path::PathBuf};

use anyhow::Result;
use clap::Args;
use pimalaya_cli::printer::Printer;
use serde::Serialize;

use io_pimdir::{PimdirAmbiguous, PimdirDangling, PimdirRefcountDrift};

use crate::cli::{StoreFlags, bytes, report};

/// How many entries of each kind the text output prints before summarising the
/// rest. The JSON output always carries them all.
const SHOWN: usize = 20;

/// Check a store's internal consistency.
///
/// Reports object rows whose body is missing, refcounts that disagree with the
/// references justifying them, rows pointing at something absent, and blob
/// files no row references (which `pimdir gc` reclaims). Reading only, unless
/// `--fix` is passed.
#[derive(Debug, Args)]
pub struct CheckCommand {
    /// Repair what can be repaired from what the store already holds.
    ///
    /// Recomputes the drifted refcounts from the pointers that justify them,
    /// and deletes the bindings whose item is gone. It destroys nothing and
    /// reclaims nothing, so it neither asks nor waits: a body is `pimdir gc`'s
    /// to take.
    #[arg(long)]
    pub fix: bool,
}

impl CheckCommand {
    /// Runs the check, and the repairs when asked.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let read = store.read()?;
        let indexed = read.indexed_hashes().map_err(report)?;
        let on_disk = read.blobs().files()?;

        let mut orphans = Vec::new();
        let mut orphan_bytes = 0;
        for blob in &on_disk {
            if !indexed.contains(&blob.hash) {
                orphan_bytes += blob.size;
                orphans.push(blob.clone());
            }
        }

        let names: BTreeSet<&String> = on_disk.iter().map(|blob| &blob.hash).collect();
        let missing: Vec<String> = indexed
            .iter()
            .filter(|hash| !names.contains(hash))
            .cloned()
            .collect();

        let drift = read.refcount_drift().map_err(report)?;
        let dangling = read.dangling().map_err(report)?;
        let ambiguous = read.ambiguous_bindings().map_err(report)?;

        let mut repaired = 0;
        let mut cleared = 0;

        if self.fix && (!drift.is_empty() || !dangling.is_empty()) {
            // NOTE: the read-only handle cannot write, and the repair takes the
            // owner role like every other write this tool makes. Dropped first,
            // so the two never hold the store at once.
            drop(read);
            let owner = store.owner()?;
            repaired = owner.recompute_refcounts().map_err(report)?;
            cleared = owner.clear_dangling_bindings().map_err(report)?;
        }

        printer.out(CheckOutput {
            orphans: orphans
                .into_iter()
                .map(|blob| OrphanBlob {
                    hash: blob.hash,
                    size: blob.size,
                    path: blob.path,
                })
                .collect(),
            orphan_bytes,
            missing,
            drift,
            dangling,
            ambiguous,
            repaired,
            cleared,
        })
    }
}

/// One blob file no object row references.
#[derive(Debug, Serialize)]
pub struct OrphanBlob {
    /// The hash its filename claims.
    pub hash: String,
    /// Its size on disk.
    pub size: u64,
    /// Where it sits.
    pub path: PathBuf,
}

/// The `check` output.
#[derive(Debug, Serialize)]
pub struct CheckOutput {
    /// Blob files no object row references.
    pub orphans: Vec<OrphanBlob>,
    /// What they weigh in total.
    pub orphan_bytes: u64,
    /// Object rows whose blob file is missing.
    pub missing: Vec<String>,
    /// Objects whose refcount disagrees with their references.
    pub drift: Vec<PimdirRefcountDrift>,
    /// Rows pointing at something absent.
    pub dangling: Vec<PimdirDangling>,
    /// Identities a source holds more than once, so their items are frozen.
    pub ambiguous: Vec<PimdirAmbiguous>,
    /// Refcounts `--fix` recomputed from the pointers justifying them.
    pub repaired: usize,
    /// Dangling bindings `--fix` cleared.
    pub cleared: usize,
}

impl CheckOutput {
    /// How many problems the check found.
    fn problems(&self) -> usize {
        self.orphans.len() + self.missing.len() + self.drift.len() + self.dangling.len()
    }
}

impl fmt::Display for CheckOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.problems() == 0 {
            return writeln!(
                f,
                "This store is consistent: no orphan blob, no refcount drift, no dangling row"
            );
        }

        if !self.orphans.is_empty() {
            writeln!(
                f,
                "{} orphan blob file(s), {} not referenced by any object row:",
                self.orphans.len(),
                bytes(self.orphan_bytes)
            )?;
            for orphan in self.orphans.iter().take(SHOWN) {
                writeln!(f, " - {} ({})", orphan.hash, bytes(orphan.size))?;
            }
            more(f, self.orphans.len())?;
            writeln!(f, "   Reclaim them with `pimdir gc`")?;
        }

        if !self.missing.is_empty() {
            writeln!(
                f,
                "{} object row(s) whose body is missing from the blob store:",
                self.missing.len()
            )?;
            for hash in self.missing.iter().take(SHOWN) {
                writeln!(f, " - {hash}")?;
            }
            more(f, self.missing.len())?;
        }

        if !self.drift.is_empty() {
            writeln!(f, "{} object(s) with a drifted refcount:", self.drift.len())?;
            for drift in self.drift.iter().take(SHOWN) {
                writeln!(
                    f,
                    " - {}: stored {}, references {}",
                    drift.hash, drift.stored, drift.expected
                )?;
            }
            more(f, self.drift.len())?;
        }

        if !self.dangling.is_empty() {
            writeln!(f, "{} dangling row(s):", self.dangling.len())?;
            for dangling in self.dangling.iter().take(SHOWN) {
                writeln!(
                    f,
                    " - {} {} points at a missing {}",
                    dangling.kind, dangling.row, dangling.target
                )?;
            }
            more(f, self.dangling.len())?;
        }

        if !self.ambiguous.is_empty() {
            writeln!(
                f,
                "{} identity/identities a source holds more than once, whose items \
                 do not sync until it holds them once again:",
                self.ambiguous.len()
            )?;
            for ambiguous in self.ambiguous.iter().take(SHOWN) {
                writeln!(
                    f,
                    " - {}/{} on {}: {} copies",
                    ambiguous.collection, ambiguous.link_id, ambiguous.source, ambiguous.copies,
                )?;
            }
            more(f, self.ambiguous.len())?;
        }

        if self.repaired > 0 || self.cleared > 0 {
            writeln!(
                f,
                "Repaired {} refcount(s) and cleared {} dangling binding(s)",
                self.repaired, self.cleared
            )?;
        }

        Ok(())
    }
}

/// States how many entries a capped list left out, so a listing is never
/// silently short.
fn more(f: &mut fmt::Formatter<'_>, total: usize) -> fmt::Result {
    if total > SHOWN {
        writeln!(
            f,
            "   … and {} more (use --json for the full list)",
            total - SHOWN
        )?;
    }
    Ok(())
}
