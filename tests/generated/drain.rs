//! # The drain against the model
//!
//! io-pimdir's owner draining a generated queue (STORAGE §15.2, §15.3,
//! SYNC §7) against `Model::drain_all`, the generator's reading of the
//! same rules: what became of each pending row (applied with its receipt's
//! `seq`, parked, or left pending), then every read of the store it leaves
//! against the oracle stated over the drained model.

use io_pimdir::client::{PimdirStore, producer::PimdirActionStatus, reader::PimdirReader};
use pimdir_generator::{oracle::Expectation, queue::RowState};
use rusqlite::types::Value;

use crate::{
    common::{Generated, SEEDS},
    reader,
};

#[test]
fn a_drained_generated_queue_leaves_what_the_model_drains_to() {
    for seed in SEEDS {
        let differences = differences(seed);
        let unknown: Vec<&String> = differences
            .iter()
            .filter(|(known, _)| known.is_none())
            .map(|(_, difference)| difference)
            .collect();
        assert!(unknown.is_empty(), "seed {seed}: {unknown:#?}");
    }
}

#[test]
#[ignore = "G6: the binding's shared body after an applied local write"]
fn a_drained_generated_queue_leaves_what_the_model_drains_to_known_included() {
    let differences: Vec<(u64, Option<&str>, String)> = SEEDS
        .into_iter()
        .flat_map(|seed| {
            differences(seed)
                .into_iter()
                .map(move |(known, difference)| (seed, known, difference))
        })
        .collect();
    assert!(differences.is_empty(), "{differences:#?}");
}

/// Every difference between io-pimdir's drain of the store at `seed` and
/// the model's, each with the known finding it stands under.
fn differences(seed: u64) -> Vec<(Option<&'static str>, String)> {
    let generated = Generated::structure(seed);
    let mut drained = generated.model.clone();
    drained.drain_all();

    let mut owner = PimdirStore::open(generated.root())
        .unwrap()
        .for_source("local");
    owner.drain().unwrap();
    drop(owner);

    let reader = PimdirReader::open(generated.root()).unwrap();
    let mut differences = Vec::new();
    for row in &generated.model.queue {
        if row.state != RowState::Pending {
            continue;
        }
        let want = &drained.queue[row.id as usize - 1].state;
        let got = reader.action_status(row.id).unwrap();
        let agree = match (want, &got) {
            (RowState::Pending, PimdirActionStatus::Pending { .. }) => true,
            (RowState::Parked(_), PimdirActionStatus::Parked { .. }) => true,
            (RowState::Applied(want), PimdirActionStatus::Applied { seq, .. }) => want == seq,
            _ => false,
        };
        if !agree {
            let difference = format!(
                "row {} {} on {}: io-pimdir {got:?}, the model {want:?}",
                row.id,
                row.action.name(),
                row.collection
            );
            differences.push((None, difference));
        }
    }

    let (_, rows) = reader::compare(&generated.spec, &drained, &generated.root(), shared);
    differences.extend(rows);
    differences
}

/// Finding G6 (spec minor, the generator's to follow): after the drain
/// applies an `update`, or an `add`, `move` or `copy` creating a pending
/// create, io-pimdir moves the acting binding's `shared_object` to the
/// body written, as SYNC §9 says of "a live upsert carrying a body"; the
/// generator keeps the body agreed before, or none, and no vector pins
/// either.
const G6: &str = "G6: the binding's shared body after an applied local write";

/// Whether a disagreement is G6: the bindings agree but on `shared_object`,
/// io-pimdir's naming a body.
fn shared(
    expectation: &Expectation,
    columns: &[&str],
    got: &[Vec<Value>],
    want: &[Vec<Value>],
) -> Option<&'static str> {
    let at = columns
        .iter()
        .position(|column| *column == "shared_object")?;
    let only = got.len() == want.len()
        && got.iter().zip(want).all(|(got, want)| {
            (0..got.len()).all(|column| column == at || got[column] == want[column])
                && got[at] != Value::Null
        });
    (expectation.statement == "storage/read/list_item_bindings" && only).then_some(G6)
}
