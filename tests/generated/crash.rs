//! # A write cut at a commit, finished on reopening
//!
//! The owner writes of tests/generated/index.rs and the index refresh, each
//! cut after every commit it makes and run again on the store reopened
//! (STORAGE §7, §8; SEARCH §4), against the run no crash cut.
//!
//! A crash is the database as its write-ahead log leaves it: the store is
//! closed before the write, so the log the write leaves holds its commits
//! alone, and the store a crash after commit `k` leaves is the store
//! before it with the log cut at the end of that commit's last frame,
//! which is what SQLite reads back on reopening. The blobs are those
//! before the write: a write removes a body only after the commit naming
//! it gone (STORAGE §5), so a crash after a commit leaves it held.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    path::Path,
};

use io_pimdir::client::{
    PimdirStore,
    search::{PimdirIndex, PimdirIndexRefreshOptions},
};
use pimdir_generator::{oracle::INSTANT, profile::Profile, spec::Spec};
use rusqlite::{Connection, types::Value};

use crate::{
    common::{Generated, SEEDS},
    index::{self, BATCH, NOW},
};

/// The writes that checkpoint the log themselves, whose cuts this test
/// cannot form from the log alone: each is cut before it ran only.
const CHECKPOINTING: [&str; 1] = ["erase"];

#[test]
fn an_owner_write_cut_after_any_commit_finishes_as_the_uncut_run() {
    for seed in SEEDS {
        let generated = Generated::structure(seed);
        let root = generated.root();
        let mut drained = generated.model.clone();
        drained.drain_all();

        let mut differences = Vec::new();
        for write in index::writes(&drained) {
            let label = write.label();
            let before = tempfile::tempdir().unwrap();
            let held = tempfile::tempdir().unwrap();
            index::copy_store(&root, &before.path().join("store"), true);

            let mut owner = PimdirStore::open(&root).unwrap().for_source("local");
            write.apply(&mut owner).unwrap();
            let log = log_of(
                &root,
                "pimdir.db",
                &before.path().join("store"),
                held.path(),
            );
            drop(owner);
            let uncut = store_rows(&root);

            let checkpointing = CHECKPOINTING.iter().any(|name| label.starts_with(name));
            assert_eq!(
                log.is_none(),
                checkpointing,
                "{label}: whether it checkpoints the log"
            );
            let ends = log.map_or(vec![0], |log| log.ends);
            for end in ends {
                let cut = cut(before.path(), held.path(), "pimdir.db-wal", end);
                let mut owner = PimdirStore::open(cut.path().join("store"))
                    .unwrap()
                    .for_source("local");
                let again = write.apply(&mut owner);
                drop(owner);
                let rows = store_rows(&cut.path().join("store"));
                if rows != uncut {
                    differences.push(format!(
                        "seed {seed} {label} cut at byte {end} of the log, run again ({again:?}): {}",
                        apart(&rows, &uncut)
                    ));
                }
            }
        }
        assert!(differences.is_empty(), "{differences:#?}");
    }
}

#[test]
fn an_index_refresh_cut_after_any_batch_finishes_as_the_uncut_refresh() {
    for seed in SEEDS {
        let spec = Spec::load_default().unwrap();
        let model = Profile::Structure.generate(&spec, seed).unwrap();
        let generated = Generated::write(spec, index::tamed(model));
        let root = generated.root();
        let mut drained = generated.model.clone();
        drained.drain_all();

        let options = PimdirIndexRefreshOptions {
            batch: BATCH,
            now: Some(NOW),
            ..Default::default()
        };
        PimdirIndex::open(&root).unwrap().refresh(&options).unwrap();

        let mut differences = Vec::new();
        for write in index::writes(&drained) {
            let label = write.label();
            let mut owner = PimdirStore::open(&root).unwrap().for_source("local");
            write.apply(&mut owner).unwrap();
            drop(owner);

            let before = tempfile::tempdir().unwrap();
            let held = tempfile::tempdir().unwrap();
            index::copy_store(&root, &before.path().join("store"), true);
            let mut uncut = PimdirIndex::open(&root).unwrap();
            uncut.refresh(&options).unwrap();
            let log = log_of(&root, "index.db", &before.path().join("store"), held.path());
            drop(uncut);
            let want = index::dump(&root);

            let Some(log) = log else {
                differences.push(format!("seed {seed} {label}: the refresh checkpointed"));
                continue;
            };
            for end in log.ends {
                let cut = cut(before.path(), held.path(), "index.db-wal", end);
                let mut index = PimdirIndex::open(cut.path().join("store")).unwrap();
                index.refresh(&options).unwrap();
                drop(index);
                let got = index::dump(&cut.path().join("store"));
                for (table, rows) in &got {
                    if want.get(table) != Some(rows) {
                        differences.push(format!(
                            "seed {seed} {label} cut at byte {end}, {table}: {rows:?}, uncut {:?}",
                            want.get(table)
                        ));
                    }
                }
            }
        }
        assert!(differences.is_empty(), "{differences:#?}");
    }
}

/// The write-ahead log a write left, and where each of its commits ends.
struct Log {
    ends: Vec<u64>,
}

/// Holds the log of the database `name` at `root` in `held` and reads where
/// each commit of it ends, after the start of the log; `None` when the write
/// checkpointed, its database no longer the one `before` holds.
fn log_of(root: &Path, name: &str, before: &Path, held: &Path) -> Option<Log> {
    if fs::read(root.join(name)).unwrap() != fs::read(before.join(name)).unwrap() {
        return None;
    }
    let wal = format!("{name}-wal");
    fs::copy(root.join(&wal), held.join(&wal)).unwrap();
    let bytes = fs::read(held.join(&wal)).unwrap();

    let word = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    let mut ends = vec![0];
    if bytes.len() < 32 {
        return Some(Log { ends });
    }
    let page = word(8) as usize;
    let salt = (word(16), word(20));
    let mut at = 32;
    while at + 24 + page <= bytes.len() && (word(at + 8), word(at + 12)) == salt {
        if word(at + 4) != 0 {
            ends.push((at + 24 + page) as u64);
        }
        at += 24 + page;
    }
    Some(Log { ends })
}

/// A copy of the store `before` holds with the log `held` holds cut at
/// byte `end`, its shared-memory index left for SQLite to build again.
fn cut(before: &Path, held: &Path, wal: &str, end: u64) -> tempfile::TempDir {
    let cut = tempfile::tempdir().unwrap();
    let store = cut.path().join("store");
    index::copy_store(&before.join("store"), &store, true);
    if end > 0 {
        fs::copy(held.join(wal), store.join(wal)).unwrap();
        OpenOptions::new()
            .write(true)
            .open(store.join(wal))
            .unwrap()
            .set_len(end)
            .unwrap();
    }
    cut
}

/// Every row of the store at `root` by table, in one order, an instant
/// SQLite stamped (a column named `_at`) as the oracle's marker, and the
/// files of its blob tree and quarantine as `files`.
pub fn store_rows(root: &Path) -> BTreeMap<String, Vec<Vec<Value>>> {
    let conn = Connection::open(root.join("pimdir.db")).unwrap();
    let names: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();

    let mut rows = BTreeMap::new();
    for name in names {
        let mut statement = conn.prepare(&format!("SELECT * FROM \"{name}\"")).unwrap();
        let instants: Vec<bool> = statement
            .column_names()
            .map(|column| column.ends_with("_at"))
            .collect();
        let mut table: Vec<Vec<Value>> = statement
            .query_map([], |row| {
                (0..instants.len())
                    .map(|at| match (instants[at], row.get(at)?) {
                        (true, Value::Text(_)) => Ok(Value::Text(String::from(INSTANT))),
                        (_, value) => Ok(value),
                    })
                    .collect()
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        table.sort_by_key(|row| format!("{row:?}"));
        rows.insert(name, table);
    }
    drop(conn);

    let mut files = Vec::new();
    for dir in ["objects", "quarantine"] {
        walk(&root.join(dir), &mut files);
    }
    files.sort_by_key(|file| format!("{file:?}"));
    rows.insert(String::from("files"), files);
    rows
}

fn walk(dir: &Path, files: &mut Vec<Vec<Value>>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        match path.is_dir() {
            true => walk(&path, files),
            false => files.push(vec![Value::Text(
                path.file_name().unwrap().to_string_lossy().into_owned(),
            )]),
        }
    }
}

/// The tables two dumps hold apart, each with the rows only one holds.
fn apart(
    got: &BTreeMap<String, Vec<Vec<Value>>>,
    want: &BTreeMap<String, Vec<Vec<Value>>>,
) -> String {
    let empty = Vec::new();
    let only = |a: &Vec<Vec<Value>>, b: &Vec<Vec<Value>>| -> Vec<Vec<Value>> {
        a.iter().filter(|row| !b.contains(row)).cloned().collect()
    };
    let tables: BTreeSet<&String> = got.keys().chain(want.keys()).collect();
    tables
        .into_iter()
        .filter_map(|table| {
            let left = got.get(table).unwrap_or(&empty);
            let right = want.get(table).unwrap_or(&empty);
            (left != right).then(|| {
                format!(
                    "{table}: cut alone {:?}, uncut alone {:?}",
                    only(left, right),
                    only(right, left)
                )
            })
        })
        .collect::<Vec<_>>()
        .join("; ")
}
