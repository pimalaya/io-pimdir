//! # A write cut at a commit, finished on reopening
//!
//! The owner writes of tests/generated/index.rs, the index refresh and every
//! engine run of the generator's histories (tests/generated/replay.rs), each
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
    fs::{self, DirBuilder, OpenOptions},
    os::unix::fs::DirBuilderExt,
    path::Path,
};

use io_pimdir::client::{
    PimdirStore,
    search::{PimdirIndex, PimdirIndexRefreshOptions},
};
use pimdir_generator::{
    engine::Run,
    history::{History, Step},
    model::Model,
    oracle::INSTANT,
    profile::Profile,
    spec::Spec,
};
use rusqlite::{Connection, types::Value};

use crate::{
    common::{Generated, SEEDS},
    index::{self, BATCH, NOW},
    replay::{self, Scripted},
};

/// The writes that checkpoint the log themselves, whose cuts this test
/// cannot form from the log alone: each is cut before it ran only.
const CHECKPOINTING: [&str; 1] = ["erase"];

#[test]
fn an_owner_write_cut_after_any_commit_finishes_as_the_uncut_run() {
    for seed in SEEDS {
        let spec = Spec::load_default().unwrap();
        let model = Profile::Structure.generate(&spec, seed).unwrap();
        let differences = owner_cuts(model);
        assert!(differences.is_empty(), "seed {seed}: {differences:#?}");
    }
}

/// Every owner write of index.rs's run over the store of `model`, cut
/// after each commit and run again, that leaves another store than the
/// uncut write.
pub fn owner_cuts(model: Model) -> Vec<String> {
    let generated = Generated::write(Spec::load_default().unwrap(), model);
    let root = generated.root();
    let mut drained = generated.model.clone();
    drained.drain_all();

    let mut differences = Vec::new();
    for write in index::writes(&drained) {
        let label = write.label();
        let before = tempfile::tempdir().unwrap();
        let held = tempfile::tempdir().unwrap();
        index::copy_store(&root, &before.path().join("store"), true);

        let mut owner = PimdirStore::open(&root)
            .unwrap()
            .for_source("local")
            .unwrap();
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
                .for_source("local")
                .unwrap();
            let again = write.apply(&mut owner);
            drop(owner);
            let rows = store_rows(&cut.path().join("store"));
            if rows != uncut {
                differences.push(format!(
                    "{label} cut at byte {end} of the log, run again ({again:?}): {}",
                    apart(&rows, &uncut)
                ));
            }
        }
    }
    differences
}

#[test]
fn an_index_refresh_cut_after_any_batch_finishes_as_the_uncut_refresh() {
    for seed in SEEDS {
        let spec = Spec::load_default().unwrap();
        let model = Profile::Structure.generate(&spec, seed).unwrap();
        let differences = index_cuts(index::tamed(model));
        assert!(differences.is_empty(), "seed {seed}: {differences:#?}");
    }
}

/// Every index refresh after an owner write of index.rs's run over the
/// store of `model`, a model [`index::tamed`] holds, cut after each batch
/// and run again, that leaves another index than the uncut refresh.
pub fn index_cuts(model: Model) -> Vec<String> {
    let generated = Generated::write(Spec::load_default().unwrap(), model);
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
        let mut owner = PimdirStore::open(&root)
            .unwrap()
            .for_source("local")
            .unwrap();
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
            differences.push(format!("{label}: the refresh checkpointed"));
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
                        "{label} cut at byte {end}, {table}: {rows:?}, uncut {:?}",
                        want.get(table)
                    ));
                }
            }
        }
    }
    differences
}

#[test]
fn an_engine_run_cut_after_any_commit_finishes_as_the_uncut_run() {
    let mut differences = Vec::new();
    for seed in SEEDS {
        for (name, history) in replay::histories(seed) {
            differences.extend(
                engine_cuts(&history)
                    .into_iter()
                    .map(|(_, text)| format!("seed {seed} {name} {text}")),
            );
        }
    }
    assert!(differences.is_empty(), "{differences:#?}");
}

/// Every run of `history` cut after each of its commits and run again
/// that leaves another store than the uncut run, each with the index of
/// its step.
pub fn engine_cuts(history: &History) -> Vec<(usize, String)> {
    let history = replay::declared(history);
    let mut revisions = BTreeMap::new();
    let mut differences = Vec::new();
    for (index, pair) in history.steps.windows(2).enumerate() {
        let (before, step) = (&pair[0], &pair[1]);
        let Some(run) = &step.run else {
            continue;
        };
        replay::remember(run, &mut revisions);
        differences.extend(
            cut_run(before, step, run, &revisions)
                .into_iter()
                .map(|text| (index + 1, format!("{}: {text}", step.label))),
        );
    }
    differences
}

/// Cuts one run after each of its commits, from the store of the step
/// before it with the step's queued rows drained, and runs it again on the
/// store reopened, its connector answering what each request asks: the
/// store it leaves is the uncut run's, or the one the uncut run and that
/// same run again leave, where the cut fell past a write leaving a change
/// owed to the next run (SYNC §5).
fn cut_run(
    before: &Step,
    step: &Step,
    run: &Run,
    revisions: &BTreeMap<(String, String), String>,
) -> Vec<String> {
    let generated = Generated::write(Spec::load_default().unwrap(), before.model.clone());
    let root = generated.root();
    let _ = replay::produce(&root, before, step);

    let start = tempfile::tempdir().unwrap();
    let held = tempfile::tempdir().unwrap();
    index::copy_store(&root, &start.path().join("store"), true);

    let keeper = Connection::open(root.join("pimdir.db")).unwrap();
    keeper
        .query_row("PRAGMA schema_version", [], |row| row.get::<_, i64>(0))
        .unwrap();
    let mut remote = Scripted::of(before, step, run, revisions, false);
    let uncut = replay::run_verb(&root, before, step, run, &mut remote);
    let log = log_of(&root, "pimdir.db", &start.path().join("store"), held.path());
    drop(keeper);
    let want = store_rows(&root);
    let next = {
        let copy = tempfile::tempdir().unwrap();
        let store = copy.path().join("store");
        index::copy_store(&root, &store, true);
        let mut remote = Scripted::of(before, step, run, revisions, true);
        let _ = replay::run_verb(&store, before, step, run, &mut remote);
        store_rows(&store)
    };

    let Some(log) = log else {
        return vec![String::from("the run checkpointed the log")];
    };
    let mut found = Vec::new();
    for end in log.ends {
        let cut = cut(start.path(), held.path(), "pimdir.db-wal", end);
        let store = cut.path().join("store");
        held_objects(&root.join("objects"), &store.join("objects"));
        let moved = round(&store, run) != round(&start.path().join("store"), run);
        let mut remote = Scripted::of(before, step, run, revisions, true).restarted(moved);
        let again = replay::run_verb(&store, before, step, run, &mut remote);
        let got = store_rows(&store);
        if got != want && got != next {
            found.push(format!(
                "cut at byte {end}, run again ({:?}, uncut {:?}): {}",
                again.map(|events| events.len()),
                uncut.as_ref().map(Vec::len),
                apart(&got, &want)
            ));
        }
    }
    found
}

/// The round the run's source holds open or last closed in the store at
/// `root`.
fn round(root: &Path, run: &Run) -> i64 {
    let conn = Connection::open(root.join("pimdir.db")).unwrap();
    conn.query_row(
        "SELECT round FROM sources WHERE collection = ?1 AND source = ?2",
        [run.collection.as_str(), run.source.as_str()],
        |row| row.get(0),
    )
    .unwrap_or(0)
}

/// Copies into `to` each blob of `from` it lacks: the bodies a run wrote
/// before the commit naming them, which a crash after any commit leaves.
fn held_objects(from: &Path, to: &Path) {
    let Ok(entries) = fs::read_dir(from) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        match path.is_dir() {
            true => {
                if !target.exists() {
                    DirBuilder::new().mode(0o700).create(&target).unwrap();
                }
                held_objects(&path, &target);
            }
            false => {
                if !target.exists() {
                    fs::copy(&path, &target).unwrap();
                }
            }
        }
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

/// The tables two dumps hold apart, each with the rows only one holds, the
/// first named `cut`, the second `uncut`.
pub fn apart(
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
