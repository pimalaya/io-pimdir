//! The `export` verb: a portable dump of a store.
//!
//! The dump is the store's own metadata plus its bodies, byte for byte: a
//! manifest naming the collections, one JSON-lines file of items per
//! collection, and a copy of the blob tree. Nothing is parsed on the way out,
//! so a dump of a mail store and a dump of an address book differ only in what
//! the bytes happen to be.
//!
//! There is deliberately no `import` beside it. The documented import path
//! (`cairn/spec/import.md`) converts Maildir, m2dir and vdir trees, and every
//! step of it reads item content (a `Message-ID` out of RFC 822 bytes, Maildir
//! flag letters, a vCard `UID`), which is the one thing this tool must not do.
//! That belongs to a per-kind importer.

use std::{
    collections::BTreeSet,
    fmt,
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use clap::Args;
use io_pimdir::PimdirBlobs;
use io_replica::{collection::ReplicaCollectionId, object::ReplicaHash};
use log::warn;
use pimalaya_cli::printer::Printer;
use serde::Serialize;
use serde_json::json;

use crate::cli::{StoreFlags, bytes, now, report};

/// How many items are read per page while dumping a collection.
const PAGE: usize = 500;

/// The dump format's own version, bumped when its shape changes.
const FORMAT_VERSION: i64 = 1;

/// Dump a store to a directory: a manifest, one JSON-lines file of items per
/// collection, and a copy of every body.
///
/// Items are dumped exactly as stored (public id, link id, flags, level, body
/// hash and raw meta), so the dump carries no interpretation of any kind. Use
/// it to snapshot a store before a risky operation, to move one between
/// machines, or to look at a store with ordinary text tools.
#[derive(Debug, Args)]
pub struct ExportCommand {
    /// Directory to write the dump into (created if missing).
    #[arg(value_name = "DIR")]
    pub dir: PathBuf,

    /// Dump the retained items too, in their own files.
    #[arg(long)]
    pub retained: bool,

    /// Dump the metadata only, without copying the bodies.
    #[arg(long)]
    pub no_objects: bool,

    /// Overwrite an existing dump in this directory.
    #[arg(long)]
    pub force: bool,
}

impl ExportCommand {
    /// Writes the dump.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let read = store.read()?;
        let blobs = store.blobs()?;

        let manifest_path = self.dir.join("manifest.json");
        if manifest_path.exists() && !self.force {
            bail!(
                "{} already holds a dump: pass --force to overwrite it",
                self.dir.display()
            );
        }
        fs::create_dir_all(self.dir.join("items"))?;

        let mut collections = Vec::new();
        let mut hashes = BTreeSet::new();
        let mut items = 0;

        for (index, collection) in read
            .list_collections()
            .map_err(report)?
            .into_iter()
            .enumerate()
        {
            // NOTE: a collection id may hold anything a mailbox name may hold,
            // slashes included, so files are numbered and the manifest carries
            // the mapping rather than escaping ids into filenames.
            let file = format!("items/{:04}.jsonl", index + 1);
            let live = self.dump_live(&read, &collection.id, &self.dir.join(&file), &mut hashes)?;

            let (retained_file, retained) = if self.retained {
                let file = format!("items/{:04}.retained.jsonl", index + 1);
                let count =
                    self.dump_retained(&read, &collection.id, &self.dir.join(&file), &mut hashes)?;
                (Some(file), count)
            } else {
                (None, 0)
            };

            items += live + retained;
            collections.push(json!({
                "id": collection.id,
                "kind": collection.kind,
                "name": collection.name,
                "parent": collection.parent,
                "color": collection.color,
                "description": collection.description,
                "sort_order": collection.sort_order,
                "generation": collection.generation,
                "items": file,
                "retained_items": retained_file,
                "live": live,
                "retained": retained,
            }));
        }

        let (objects, object_bytes, missing) = if self.no_objects {
            (0, 0, Vec::new())
        } else {
            self.dump_objects(&blobs, &hashes)?
        };

        let collection_count = collections.len();
        let manifest = json!({
            "format": "pimdir-export",
            "format_version": FORMAT_VERSION,
            "exported_at": now(),
            "store": store.dir().display().to_string(),
            "sources": read.distinct_sources().map_err(report)?,
            "collections": collections,
            "objects": objects,
            "object_bytes": object_bytes,
            "missing_objects": missing,
        });
        fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;

        printer.out(ExportOutput {
            dir: self.dir,
            collections: collection_count,
            items,
            objects,
            bytes: object_bytes,
            missing,
        })
    }

    /// Dumps a collection's live items, page by page, returning how many.
    fn dump_live(
        &self,
        read: &io_pimdir::PimdirStore,
        collection: &str,
        path: &Path,
        hashes: &mut BTreeSet<String>,
    ) -> Result<u64> {
        let mut file = BufWriter::new(File::create(path)?);
        let mut after: Option<String> = None;
        let mut count = 0;

        loop {
            let page = read
                .list_items(collection, after.as_deref(), PAGE)
                .map_err(report)?;
            if page.is_empty() {
                break;
            }

            for item in &page {
                if let Some(hash) = &item.object {
                    hashes.insert(hash.0.clone());
                }
                let line = json!({
                    "collection": collection,
                    "seq": item.seq,
                    "link_id": item.link_id.0,
                    "flags": item.flags.known(),
                    "level": level(item.level),
                    "object": item.object.as_ref().map(|hash| hash.0.clone()),
                    "meta": item.meta.as_ref().map(|meta| meta.0.clone()),
                });
                writeln!(file, "{line}")?;
                count += 1;
            }

            after = page.last().map(|item| item.link_id.0.clone());
        }

        file.flush()?;
        Ok(count)
    }

    /// Dumps a collection's retained items, page by page, returning how many.
    fn dump_retained(
        &self,
        read: &io_pimdir::PimdirStore,
        collection: &str,
        path: &Path,
        hashes: &mut BTreeSet<String>,
    ) -> Result<u64> {
        let id = ReplicaCollectionId(collection.to_string());
        let mut file = BufWriter::new(File::create(path)?);
        let mut after: Option<i64> = None;
        let mut count = 0;

        loop {
            let page = read.list_retained(&id, after, PAGE).map_err(report)?;
            if page.is_empty() {
                break;
            }

            for item in &page {
                if let Some(hash) = &item.object_hash {
                    hashes.insert(hash.clone());
                }
                let line = json!({
                    "collection": collection,
                    "seq": item.seq,
                    "link_id": item.link_id,
                    "flags": item.flags.known(),
                    "level": level(item.level),
                    "object": item.object_hash,
                    "meta": item.meta,
                    "retained_at": item.retained_at,
                    "retained_by": item.retained_by,
                });
                writeln!(file, "{line}")?;
                count += 1;
            }

            after = page.last().map(|item| item.seq);
        }

        file.flush()?;
        Ok(count)
    }

    /// Copies every referenced body into the dump, returning how many objects
    /// were copied, their total size, and the hashes whose body was missing.
    fn dump_objects(
        &self,
        blobs: &PimdirBlobs,
        hashes: &BTreeSet<String>,
    ) -> Result<(u64, u64, Vec<String>)> {
        let mut copied = 0;
        let mut total = 0;
        let mut missing = Vec::new();

        for hash in hashes {
            let Some(mut reader) = blobs.reader(&ReplicaHash(hash.clone()))? else {
                warn!("the body of object {hash} is missing from the blob store");
                missing.push(hash.clone());
                continue;
            };

            let path = self.object_path(hash);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = File::create(&path)?;
            total += io::copy(&mut reader, &mut file)?;
            file.flush()?;
            copied += 1;
        }

        Ok((copied, total, missing))
    }

    /// A body's path inside the dump, mirroring the store's own sharding.
    fn object_path(&self, hash: &str) -> PathBuf {
        let objects = self.dir.join("objects");
        if hash.len() >= 4 {
            objects.join(&hash[0..2]).join(&hash[2..4]).join(hash)
        } else {
            objects.join(hash)
        }
    }
}

/// The detail ladder as its lowercase name (the dump never carries the column
/// integer, which is an implementation detail of the schema).
fn level(level: io_replica::placement::ReplicaLevel) -> &'static str {
    match level {
        io_replica::placement::ReplicaLevel::Probed => "probed",
        io_replica::placement::ReplicaLevel::Meta => "meta",
        io_replica::placement::ReplicaLevel::Full => "full",
    }
}

/// The `export` output.
#[derive(Debug, Serialize)]
pub struct ExportOutput {
    /// Where the dump was written.
    pub dir: PathBuf,
    /// How many collections it describes.
    pub collections: usize,
    /// How many items it holds.
    pub items: u64,
    /// How many bodies were copied.
    pub objects: u64,
    /// What those bodies weigh.
    pub bytes: u64,
    /// Hashes whose body was missing from the store.
    pub missing: Vec<String>,
}

impl fmt::Display for ExportOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Exported {} collection(s), {} item(s) and {} object(s) ({}) to {}",
            self.collections,
            self.items,
            self.objects,
            bytes(self.bytes),
            self.dir.display()
        )?;

        if !self.missing.is_empty() {
            writeln!(
                f,
                "{} object(s) had no body in the blob store and were left out; run `pimdir check`",
                self.missing.len()
            )?;
        }

        Ok(())
    }
}
