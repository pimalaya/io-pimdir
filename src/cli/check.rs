//! The `check` verb: diagnosis, and the repairs that need no guessing.
//!
//! It reports what should not happen. A refcount is maintained
//! incrementally, so a bug or a foreign writer could leave drift between
//! a count and the references justifying it; foreign keys are enforced
//! only when the writer enabled them, so a dangling row is conceivable;
//! an object row whose blob is missing is a read that will fail. An
//! orphan blob is the benign one, a file no row references, and `pimdir
//! gc` is what takes it: this verb reclaims nothing.
//!
//! `--fix` repairs, which is not reclaiming: it recomputes the drifted
//! refcounts from the pointers that justify them, and clears the
//! bindings whose item is gone. Both recover a fact the store already
//! holds. Nothing else is touched, a wrong repair being worse than a
//! reported inconsistency.

use std::{collections::BTreeSet, fmt, path::PathBuf};

use anyhow::Result;
use clap::Args;
use io_pimdir::client::blobs::PimdirBlobStrayKind;
use pimalaya_cli::{printer::Printer, sanitize};
use schemars::JsonSchema;
use serde::Serialize;

use crate::cli::{
    StoreFlags, bytes,
    error::{partial, report},
};

/// How many entries of each kind the text output prints before summarising the
/// rest. The JSON output always carries them all.
const SHOWN: usize = 20;

/// Check a store's internal consistency.
///
/// Reports object rows whose body is missing, refcounts that disagree with the
/// references justifying them, rows pointing at something absent, blob files no
/// row references (which `pimdir gc` reclaims), entries of objects/ that are no
/// body (left in place) and the files quarantine/ holds. Reading only, unless
/// `--fix` is passed.
///
/// Exits 2 once the result is printed when it found a problem it left
/// unrepaired: anything but the drift and the dangling bindings `--fix`
/// repairs.
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
        let blobs = read.blobs();
        let scan = blobs.scan()?;
        let quarantined = blobs.quarantined()?;
        let on_disk = scan.files;

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

        let drift: Vec<RefcountDrift> = read
            .refcount_drift()
            .map_err(report)?
            .into_iter()
            .map(|drift| RefcountDrift {
                hash: drift.hash,
                stored: drift.stored,
                expected: drift.expected,
            })
            .collect();
        let dangling: Vec<DanglingRow> = read
            .dangling()
            .map_err(report)?
            .into_iter()
            .map(|dangling| DanglingRow {
                kind: dangling.kind.to_string(),
                row: dangling.row,
                target: dangling.target,
            })
            .collect();
        let minted: Vec<MintedKeys> = read
            .minted_keys()
            .map_err(report)?
            .into_iter()
            .map(|minted| MintedKeys {
                collection: minted.collection,
                items: minted.items,
            })
            .collect();

        let mut repaired = 0;
        let mut cleared = 0;

        if self.fix && (!drift.is_empty() || !dangling.is_empty()) {
            // NOTE: the read-only handle cannot write, and the repair
            // takes the owner role like every other write this tool
            // makes. Dropped first, so the two never hold it at once.
            drop(read);
            let owner = store.owner()?;
            repaired = owner.recompute_refcounts().map_err(report)?;
            cleared = owner.clear_dangling_bindings().map_err(report)?;
        }

        // NOTE: `--fix` repairs the drift and the dangling bindings; the
        // rest is gc's, quarantine's or a resync's to settle.
        let unrepaired = !orphans.is_empty()
            || !scan.strays.is_empty()
            || !quarantined.is_empty()
            || !missing.is_empty()
            || (!self.fix && (!drift.is_empty() || !dangling.is_empty()));

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
            strays: scan
                .strays
                .into_iter()
                .map(|stray| StrayEntry {
                    kind: stray_kind(stray.kind),
                    path: stray.path,
                })
                .collect(),
            quarantined,
            drift,
            dangling,
            minted,
            repaired,
            cleared,
        })?;

        match unrepaired {
            true => Err(partial()),
            false => Ok(()),
        }
    }
}

/// One blob file no object row references.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OrphanBlob {
    /// The hash its filename claims.
    pub hash: String,
    /// Its size on disk.
    pub size: u64,
    /// Where it sits.
    pub path: PathBuf,
}

/// One entry of objects/ that is no body, which the collector leaves in
/// place (STORAGE §5).
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StrayEntry {
    /// What it is: `temporary`, `link` or `foreign`.
    pub kind: &'static str,
    /// Where it sits.
    pub path: PathBuf,
}

/// One object whose refcount disagrees with the references justifying it.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RefcountDrift {
    /// The object's hash.
    pub hash: String,
    /// The refcount the row stores.
    pub stored: i64,
    /// The references actually pointing at it.
    pub expected: i64,
}

/// One row pointing at something absent.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DanglingRow {
    /// The table the row belongs to.
    pub kind: String,
    /// The row, as its key.
    pub row: String,
    /// What it points at.
    pub target: String,
}

/// The minted keys one collection holds.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MintedKeys {
    /// The collection.
    pub collection: String,
    /// How many of its items sit under a minted key.
    pub items: i64,
}

/// The `check` output.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CheckOutput {
    /// Blob files no object row references.
    pub orphans: Vec<OrphanBlob>,
    /// What they weigh in total.
    pub orphan_bytes: u64,
    /// Entries of objects/ that are no body: a writer's temporary, a link,
    /// a name or a position that is not the store's.
    pub strays: Vec<StrayEntry>,
    /// Files that failed their name, moved into quarantine/ by an owner and
    /// never read again; only the user empties it.
    pub quarantined: Vec<PathBuf>,
    /// Object rows whose blob file is missing.
    pub missing: Vec<String>,
    /// Objects whose refcount disagrees with their references.
    pub drift: Vec<RefcountDrift>,
    /// Rows pointing at something absent.
    pub dangling: Vec<DanglingRow>,
    /// Minted keys per collection, the second copies of an identity a
    /// source holds twice. Informational, never a defect.
    pub minted: Vec<MintedKeys>,
    /// Refcounts `--fix` recomputed from the pointers justifying them.
    pub repaired: usize,
    /// Dangling bindings `--fix` cleared.
    pub cleared: usize,
}

impl CheckOutput {
    /// How many problems the check found.
    fn problems(&self) -> usize {
        self.orphans.len()
            + self.strays.len()
            + self.quarantined.len()
            + self.missing.len()
            + self.drift.len()
            + self.dangling.len()
    }
}

impl fmt::Display for CheckOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.problems() == 0 {
            writeln!(
                f,
                "This store is consistent: no orphan blob, no refcount drift, no dangling row"
            )?;
        }

        if !self.orphans.is_empty() {
            writeln!(
                f,
                "{} orphan blob file(s), {} not referenced by any object row:",
                self.orphans.len(),
                bytes(self.orphan_bytes)
            )?;
            for orphan in self.orphans.iter().take(SHOWN) {
                let hash = sanitize::text(&orphan.hash);
                writeln!(f, " - {hash} ({})", bytes(orphan.size))?;
            }
            more(f, self.orphans.len())?;
            writeln!(f, "   Reclaim them with `pimdir gc`")?;
        }

        if !self.strays.is_empty() {
            writeln!(
                f,
                "{} entry(ies) under objects/ that are no body, left in place:",
                self.strays.len()
            )?;
            for stray in self.strays.iter().take(SHOWN) {
                let path = stray.path.to_string_lossy();
                let path = sanitize::text(&path);
                writeln!(f, " - {path} ({})", stray.kind)?;
            }
            more(f, self.strays.len())?;
        }

        if !self.quarantined.is_empty() {
            writeln!(
                f,
                "{} file(s) in quarantine/ that failed their name, kept for recovery:",
                self.quarantined.len()
            )?;
            for path in self.quarantined.iter().take(SHOWN) {
                let path = path.to_string_lossy();
                let path = sanitize::text(&path);
                writeln!(f, " - {path}")?;
            }
            more(f, self.quarantined.len())?;
        }

        if !self.missing.is_empty() {
            writeln!(
                f,
                "{} object row(s) whose body is missing from the blob store:",
                self.missing.len()
            )?;
            for hash in self.missing.iter().take(SHOWN) {
                let hash = sanitize::text(hash);
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
                    sanitize::text(&drift.hash),
                    drift.stored,
                    drift.expected
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
                    sanitize::text(&dangling.kind),
                    sanitize::text(&dangling.row),
                    sanitize::text(&dangling.target)
                )?;
            }
            more(f, self.dangling.len())?;
        }

        // NOTE: printed under the problems, and counted in none of them:
        // a minted key is an identity a source hands over twice, stored
        // as the second item it is, which the store neither repairs nor
        // judges (spec §9). What it is worth saying is the count, since a
        // collection whose count climbs every sync is a source renaming
        // the same duplicate.
        if !self.minted.is_empty() {
            writeln!(
                f,
                "Minted keys, the second copies of an identity a source holds twice:"
            )?;
            for minted in self.minted.iter().take(SHOWN) {
                let collection = sanitize::text(&minted.collection);
                writeln!(f, " - {collection}: {} item(s)", minted.items)?;
            }
            more(f, self.minted.len())?;
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

/// A stray entry's kind as the output names it.
fn stray_kind(kind: PimdirBlobStrayKind) -> &'static str {
    match kind {
        PimdirBlobStrayKind::Temporary => "temporary",
        PimdirBlobStrayKind::Link => "link",
        PimdirBlobStrayKind::Foreign => "foreign",
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::check::*;

    #[test]
    fn the_findings_print_what_the_store_names_sanitized() {
        let output = CheckOutput {
            orphans: Vec::new(),
            orphan_bytes: 0,
            strays: vec![StrayEntry {
                kind: "foreign",
                path: PathBuf::from("objects/\u{1b}]52;c;aGk=\u{7}"),
            }],
            quarantined: Vec::new(),
            missing: Vec::new(),
            drift: Vec::new(),
            dangling: vec![DanglingRow {
                kind: String::from("binding"),
                row: String::from("local/1\u{1b}[2J"),
                target: String::from("item 7"),
            }],
            minted: vec![MintedKeys {
                collection: String::from("INBOX\u{202e}"),
                items: 2,
            }],
            repaired: 0,
            cleared: 0,
        };

        let expected = "\
1 entry(ies) under objects/ that are no body, left in place:
 - objects/\u{fffd}]52;c;aGk=\u{fffd} (foreign)
1 dangling row(s):
 - binding local/1\u{fffd}[2J points at a missing item 7
Minted keys, the second copies of an identity a source holds twice:
 - INBOX\u{fffd}: 2 item(s)
";
        assert_eq!(output.to_string(), expected);
    }
}
