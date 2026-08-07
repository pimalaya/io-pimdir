//! The `check` verb: the store's internal consistency, and the one repair that
//! is safe to automate.
//!
//! Three inconsistencies are possible by design or by accident. Blob files are
//! unlinked only after the transaction that dropped their rows commits, so a
//! crash in between leaves an **orphan blob**: a file no row references, which
//! nothing else cleans. A refcount is maintained incrementally, so a bug or a
//! foreign writer could leave **drift** between a count and the references that
//! justify it. Foreign keys are enforced only when the writer enabled them, so
//! a **dangling** row is conceivable. All three are reported; only orphan blobs
//! are reclaimable without guessing.

use std::{
    collections::BTreeSet,
    fmt, fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::Result;
use clap::Args;
use log::debug;
use pimalaya_cli::printer::Printer;
use serde::Serialize;

use crate::cli::{
    StoreFlags, bytes, confirm,
    db::{PimdirDangling, PimdirRefcountDrift},
};

/// How many entries of each kind the text output prints before summarising the
/// rest. The JSON output always carries them all.
const SHOWN: usize = 20;

/// Check a store's internal consistency.
///
/// Reports blob files no row references (a crash can leave them and nothing
/// else cleans them), object rows whose body is missing, refcounts that
/// disagree with the references justifying them, and rows pointing at something
/// absent. Reading only, unless `--fix` is passed.
#[derive(Debug, Args)]
pub struct CheckCommand {
    /// Delete the orphan blob files the check found.
    ///
    /// Only orphan files older than the grace period are deleted, and nothing
    /// else is touched: refcount drift and dangling rows are reported, never
    /// repaired, because a wrong repair is worse than a reported inconsistency.
    #[arg(long)]
    pub fix: bool,

    /// Leave orphan files younger than this alone (`1h`, `30m`, `7d`).
    ///
    /// A body is written to the blob store before the row that references it,
    /// so a file that has just appeared may belong to a write in flight. The
    /// grace period is what keeps `--fix` from deleting it.
    #[arg(long, value_name = "DURATION", default_value = "1h")]
    pub grace: humantime::Duration,

    /// Do not ask for confirmation before deleting orphan files.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

impl CheckCommand {
    /// Runs the check, and the orphan sweep when asked.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let db = store.db()?;
        let indexed = db.hashes()?;
        let on_disk = blob_files(&store.dir().join("objects"))?;

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

        let drift = db.refcount_drift()?;
        let dangling = db.dangling()?;

        let mut removed = 0;
        let mut reclaimed = 0;

        if self.fix && !orphans.is_empty() {
            let cutoff = SystemTime::now()
                .checked_sub(*self.grace)
                .unwrap_or(SystemTime::UNIX_EPOCH);
            let stale: Vec<&BlobFile> = orphans
                .iter()
                .filter(|blob| blob.modified.is_none_or(|modified| modified < cutoff))
                .collect();

            if stale.is_empty() {
                debug!("every orphan blob is younger than the grace period, nothing to delete");
            } else {
                let total: u64 = stale.iter().map(|blob| blob.size).sum();
                confirm(
                    printer,
                    self.yes,
                    &format!(
                        "Delete {} orphan blob file(s), reclaiming {}?",
                        stale.len(),
                        bytes(total)
                    ),
                )?;

                for blob in stale {
                    fs::remove_file(&blob.path)?;
                    removed += 1;
                    reclaimed += blob.size;
                }
            }
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
            removed,
            reclaimed,
        })
    }
}

/// One file found in the blob directory.
#[derive(Clone, Debug)]
struct BlobFile {
    hash: String,
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
}

/// Every blob file under `root`, walking the two-level sharding (and the flat
/// fallback for very short hashes). Temporary files (a leading dot) are skipped:
/// they belong to a writer that has not committed.
fn blob_files(root: &Path) -> Result<Vec<BlobFile>> {
    let mut files = Vec::new();
    if !root.is_dir() {
        return Ok(files);
    }
    walk(root, &mut files)?;
    Ok(files)
}

/// Recurses one directory of the blob tree.
fn walk(dir: &Path, files: &mut Vec<BlobFile>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }

        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            walk(&entry.path(), files)?;
        } else if metadata.is_file() {
            files.push(BlobFile {
                hash: name,
                path: entry.path(),
                size: metadata.len(),
                modified: metadata.modified().ok(),
            });
        }
    }

    Ok(())
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
    /// Orphan files deleted by `--fix`.
    pub removed: usize,
    /// Bytes those files freed.
    pub reclaimed: u64,
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
            if self.removed == 0 {
                writeln!(f, "   Reclaim them with `pimdir check --fix`")?;
            }
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

        if self.removed > 0 {
            writeln!(
                f,
                "Deleted {} orphan blob file(s), reclaiming {}",
                self.removed,
                bytes(self.reclaimed)
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
