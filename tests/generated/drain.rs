//! # The drain against the model
//!
//! io-pimdir's owner draining a generated queue (STORAGE §15.2, §15.3,
//! SYNC §7) against `Model::drain_all`, the generator's reading of the
//! same rules: what became of each pending row (applied with its receipt's
//! `seq`, parked, or left pending), then every read of the store it leaves
//! against the oracle stated over the drained model.

use io_pimdir::client::{PimdirStore, producer::PimdirActionStatus, reader::PimdirReader};
use pimdir_generator::{model::Model, profile::Profile, queue::RowState, shrink::Kept, spec::Spec};

use crate::{
    common::{Generated, SEEDS},
    reader, scale,
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

/// A mail drained while a card holds its sender's address keeps its
/// `sender` reference once a later row takes the address off the card: a
/// reference is never recomputed and an address removed removes nothing
/// (STORAGE §14.2). Campaign-2's structure seed 2047, shrunk.
#[test]
fn a_sender_reference_outlives_the_address_it_was_recorded_by() {
    let kept = Kept {
        collections: ["work/Sent Items", "work/contacts", "work/holidays"]
            .into_iter()
            .map(String::from)
            .collect(),
        items: vec![
            ("work/contacts".into(), "52581e03-80@pimdir.test".into()),
            ("work/holidays".into(), "71d23310-183@pimdir.test".into()),
        ],
        queue: vec![14, 17],
        capabilities: false,
        references: false,
    };
    let spec = Spec::load_default().unwrap();
    let model = kept.apply(&Profile::Structure.generate(&spec, 2047).unwrap());
    let differences = differences_of(model);
    let known = differences
        .iter()
        .filter(|(known, _)| known.is_some())
        .count();
    let unknown: Vec<&String> = differences
        .iter()
        .filter(|(known, _)| known.is_none())
        .map(|(_, difference)| difference)
        .collect();
    assert!(unknown.is_empty(), "{unknown:#?}");
    assert!(known > 0, "the reference the add recorded is read");
}

/// A card updated to an address another card already held gets its
/// `sender` reference from the mail of that address, the other card none:
/// the rule runs after the write that changed what it reads (STORAGE
/// §14.2), and no write touched the other card or the mail since the store
/// was written with no reference. Campaign-5's structure seed 5036, shrunk.
#[test]
fn a_sender_reference_is_owed_only_after_a_write_at_one_end() {
    let kept = Kept {
        collections: ["work/Sent Items", "work/contacts"]
            .into_iter()
            .map(String::from)
            .collect(),
        items: vec![
            ("work/Sent Items".into(), "invoice.31@example.net".into()),
            ("work/contacts".into(), "045cce6c-80@pimdir.test".into()),
            ("work/contacts".into(), "de73b24c-90@pimdir.test".into()),
        ],
        queue: vec![19],
        capabilities: false,
        references: false,
    };
    let spec = Spec::load_default().unwrap();
    let model = kept.apply(&Profile::Structure.generate(&spec, 5036).unwrap());
    let differences = differences_of(model);
    let known = differences
        .iter()
        .filter(|(known, _)| known.is_some())
        .count();
    let unknown: Vec<&String> = differences
        .iter()
        .filter(|(known, _)| known.is_none())
        .map(|(_, difference)| difference)
        .collect();
    assert!(unknown.is_empty(), "{unknown:#?}");
    assert!(known > 0, "the reference the update recorded is read");
}

/// Every difference between io-pimdir's drain of the store at `seed` and
/// the model's, each with the known finding it stands under.
fn differences(seed: u64) -> Vec<(Option<&'static str>, String)> {
    differences_of(Generated::structure(seed).model)
}

/// Every difference between io-pimdir's drain of the store of `model` and
/// the model's, a `sender` reference the owner may record over the drain
/// known (`scale::drained_senders`, `scale::sender`).
pub fn differences_of(model: Model) -> Vec<(Option<&'static str>, String)> {
    let generated = Generated::write(Spec::load_default().unwrap(), model);
    let recorded = scale::drained_senders(&generated.model);
    let mut drained = generated.model.clone();
    drained.drain_all();
    let rule = scale::owed(&drained, &generated.model);

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
        |expectation, columns, got, want| {
            scale::sender(&rule, &recorded, expectation, columns, got, want)
        },
    );
    differences.extend(rows);
    differences
}
