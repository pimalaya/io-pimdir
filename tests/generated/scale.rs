//! # The scale profiles
//!
//! The generator's count profile (many small items in hundreds of
//! collections, a mail tree six deep) and bytes profile (large bodies, one
//! past 25 MB and one past 100 MB at full size), each given a queue of
//! flag changes, removes and moves over its mail: io-pimdir's reader against
//! the oracle with the queue pending, its drain against the model's, and
//! its index refreshed against one rebuilt. Debug runs a small store of
//! each; the profiles' own sizes are ignored tests for a release build.

use std::collections::BTreeSet;

use io_pimdir::client::{
    PimdirStore,
    search::{PimdirIndex, PimdirIndexRefreshOptions},
};
use pimdir_generator::{
    model::{Kind, Model, State},
    oracle::Expectation,
    profile::{self, Profile},
    queue::{Action, RowState},
    spec::Spec,
};
use rusqlite::types::Value;

use crate::{
    common::Generated,
    index::{self, BATCH, NOW},
    reader,
};

/// Finding G18 (the generator's): a `move` or a `copy` of an item in a
/// collection no source syncs is applied by `Model::apply`, where STORAGE
/// §15.3 parks it, there being no binding to carry. The drained model this
/// test compares with parks it as the spec does.
const G18: &str = "G18: a move out of a local collection parks";

/// Finding G20 (implementation, minor): a `remove` of an item no source
/// binds retains it with `retained_by` the drain handle's own source, a
/// name of the caller's choosing, where the column names the source whose
/// removal retired it (STORAGE §13), none here.
const G20: &str = "G20: retained_by names the drain's source for an unbound item";

/// Finding G21 (spec minor, a MAY): io-pimdir records the automatic
/// references of STORAGE §14.2 (`sender`) after the drain writes a message,
/// which an owner MAY do; the model applies no rule, so the oracle cannot
/// state the references an owner's write leaves.
const G21: &str = "G21: automatic references an owner MAY record";

/// The seeds the scale tests run, fewer than the suite's: each store is
/// many times the structure profile's.
const SEEDS: [u64; 2] = [7, 42];

/// The items of a count store in a debug build.
const ITEMS: usize = 600;

/// What every size of a bytes store is divided by in a debug build.
const SCALE: usize = 4_096;

#[test]
fn a_small_count_store_reads_drains_and_indexes_as_its_model_states() {
    for seed in SEEDS {
        let unknown = unknown(differences(profile::count(seed, ITEMS)));
        assert!(unknown.is_empty(), "seed {seed}: {unknown:#?}");
    }
}

#[test]
fn a_small_bytes_store_reads_drains_and_indexes_as_its_model_states() {
    for seed in SEEDS {
        let unknown = unknown(differences(profile::bytes(seed, SCALE)));
        assert!(unknown.is_empty(), "seed {seed}: {unknown:#?}");
    }
}

#[test]
#[ignore = "G20 and G21: the scale findings of the generated suite"]
fn a_small_store_reads_drains_and_indexes_as_its_model_states_known_included() {
    let differences: Vec<(Option<&str>, String)> = SEEDS
        .into_iter()
        .flat_map(|seed| {
            let mut found = differences(profile::count(seed, ITEMS));
            found.extend(differences(profile::bytes(seed, SCALE)));
            found
        })
        .collect();
    assert!(differences.is_empty(), "{differences:#?}");
}

#[test]
#[ignore = "the count profile's own size, for a release build"]
fn a_count_store_reads_drains_and_indexes_as_its_model_states() {
    let spec = Spec::load_default().unwrap();
    let model = Profile::Count.generate(&spec, SEEDS[0]).unwrap();
    let unknown = unknown(differences(model));
    assert!(unknown.is_empty(), "{unknown:#?}");
}

#[test]
#[ignore = "the bytes profile's own size, for a release build"]
fn a_bytes_store_reads_drains_and_indexes_as_its_model_states() {
    let spec = Spec::load_default().unwrap();
    let model = Profile::Bytes.generate(&spec, SEEDS[0]).unwrap();
    let unknown = unknown(differences(model));
    assert!(unknown.is_empty(), "{unknown:#?}");
}

fn unknown(differences: Vec<(Option<&'static str>, String)>) -> Vec<String> {
    differences
        .into_iter()
        .filter(|(known, _)| known.is_none())
        .map(|(_, text)| text)
        .collect()
}

/// Every disagreement over `model` given its queue, each with the known
/// finding it stands under: the reads with the queue pending, the reads
/// once io-pimdir drained it, and the index refreshed against one rebuilt.
fn differences(model: Model) -> Vec<(Option<&'static str>, String)> {
    let spec = Spec::load_default().unwrap();
    let model = queued(model);
    let mut drained = model.clone();
    for row in &model.queue {
        let unbound = model.collections[&row.collection].sources.is_empty();
        if unbound && matches!(row.action, Action::Move { .. } | Action::Copy { .. }) {
            drained.settle(row.id, RowState::Parked(String::from(G18)));
        }
    }
    drained.drain_all();
    let generated = Generated::write(Spec::load_default().unwrap(), model.clone());
    let root = generated.root();

    let mut differences = Vec::new();
    let (_, read) = reader::compare(&spec, &model, &root, known);
    differences.extend(
        read.into_iter()
            .map(|(known, text)| (known, format!("pending: {text}"))),
    );

    let mut owner = PimdirStore::open(&root)
        .unwrap()
        .for_source("local")
        .unwrap();
    owner.drain().unwrap();
    drop(owner);
    let (_, read) = reader::compare(&spec, &drained, &root, known);
    differences.extend(
        read.into_iter()
            .map(|(known, text)| (known, format!("drained: {text}"))),
    );

    let options = PimdirIndexRefreshOptions {
        batch: BATCH * 100,
        now: Some(NOW),
        ..Default::default()
    };
    PimdirIndex::open(&root).unwrap().refresh(&options).unwrap();
    let refreshed = index::dump(&root);
    let rebuilt = index::rebuilt(&root, &options);
    for (table, rows) in &refreshed {
        if rebuilt.get(table) != Some(rows) {
            differences.push((
                None,
                format!(
                    "index, {table}: {} rows refreshed, {} rebuilt",
                    rows.len(),
                    rebuilt.get(table).map_or(0, Vec::len)
                ),
            ));
        }
    }
    differences
}

/// The known finding a disagreement over the drained store stands under:
/// G20 for a retained row read alike but for `retained_by`, G21 for the
/// references io-pimdir holds beyond the oracle's, every one `auto`.
fn known(
    expectation: &Expectation,
    columns: &[&str],
    got: &[Vec<Value>],
    want: &[Vec<Value>],
) -> Option<&'static str> {
    let statement = expectation.statement.as_str();
    if statement == "storage/read/list_retained_page" {
        let at = columns.iter().position(|column| *column == "retained_by")?;
        let alike = got.len() == want.len()
            && got.iter().zip(want).all(|(got, want)| {
                (0..got.len()).all(|column| column == at || got[column] == want[column])
            });
        return alike.then_some(G20);
    }
    if matches!(
        statement,
        "storage/read/references_from" | "storage/read/references_to"
    ) {
        let origin = columns.iter().position(|column| *column == "origin")?;
        let auto = Value::Text(String::from("auto"));
        let added = want.iter().all(|row| got.contains(row))
            && got
                .iter()
                .filter(|row| !want.contains(row))
                .all(|row| row[origin] == auto);
        return added.then_some(G21);
    }
    None
}

/// `model` with a producer's rows over its live mail: a flag change on one
/// item in 7 stating the set it read, a remove on one in 13 and a move into
/// its account's archive on one in 11 holding its body, each pending. A
/// move of a bodiless item is left out: finding G19 (the generator's),
/// `Model::apply` parks it where SYNC §7 lets it through, its binding based
/// and its source syncing the target, as io-pimdir does.
fn queued(mut model: Model) -> Model {
    let mail: Vec<(String, i64, Option<BTreeSet<String>>, bool)> = model
        .items
        .iter()
        .filter(|item| item.state == State::Live)
        .filter(|item| model.collections[&item.collection].kind == Some(Kind::Mail))
        .map(|item| {
            let held = item.object.is_some();
            (item.collection.clone(), item.seq, item.flags.clone(), held)
        })
        .collect();
    for (index, (collection, seq, flags, held)) in mail.into_iter().enumerate() {
        let account = collection.split('/').next().unwrap_or_default();
        let archive = format!("{account}/Archive");
        let action = match index {
            _ if index % 7 == 0 => {
                let read = flags.unwrap_or_default();
                let mut seen = read.clone();
                seen.insert(String::from("\\Seen"));
                Action::SetFlags {
                    seq,
                    flags: seen,
                    base: Some(read),
                }
            }
            _ if index % 13 == 0 => Action::Remove {
                seq,
                if_match: None,
            },
            _ if index % 11 == 0
                && collection != archive
                && held
                && model.collections.contains_key(&archive) =>
            {
                Action::Move { seq, to: archive }
            }
            _ => continue,
        };
        model.enqueue("himalaya", &collection, action);
    }
    model
}
