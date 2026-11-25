//! # The drain against the model
//!
//! io-pimdir's owner draining a generated queue (STORAGE §15.2, §15.3,
//! SYNC §7) against `Model::drain_all`, the generator's reading of the
//! same rules: what became of each pending row (applied with its receipt's
//! `seq`, parked, or left pending), then every read of the store it leaves
//! against the oracle stated over the drained model.

use io_pimdir::client::{PimdirStore, producer::PimdirActionStatus, reader::PimdirReader};
use pimdir_generator::queue::RowState;

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

/// Every difference between io-pimdir's drain of the store at `seed` and
/// the model's, each with the known finding it stands under.
fn differences(seed: u64) -> Vec<(Option<&'static str>, String)> {
    let generated = Generated::structure(seed);
    let mut drained = generated.model.clone();
    drained.drain_all();

    let mut owner = PimdirStore::open(generated.root())
        .unwrap()
        .for_source("local")
        .unwrap();
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

    let (_, rows) = reader::compare(
        &generated.spec,
        &drained,
        &generated.root(),
        |_, _, _, _| None,
    );
    differences.extend(rows);
    differences
}
