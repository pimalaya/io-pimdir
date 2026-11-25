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

/// What the oracle accepts beside its own answer: the automatic `sender`
/// references an owner MAY record (STORAGE §14.2), which the model states
/// none of.
const SENDER: &str = "the sender references an owner may record";

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
        let differences = differences(profile::count(seed, ITEMS));
        assert!(differences.is_empty(), "seed {seed}: {differences:#?}");
    }
}

#[test]
fn a_small_bytes_store_reads_drains_and_indexes_as_its_model_states() {
    for seed in SEEDS {
        let differences = differences(profile::bytes(seed, SCALE));
        assert!(differences.is_empty(), "seed {seed}: {differences:#?}");
    }
}

#[test]
#[ignore = "the count profile's own size, for a release build"]
fn a_count_store_reads_drains_and_indexes_as_its_model_states() {
    let spec = Spec::load_default().unwrap();
    let model = Profile::Count.generate(&spec, SEEDS[0]).unwrap();
    let differences = differences(model);
    assert!(differences.is_empty(), "{differences:#?}");
}

#[test]
#[ignore = "the bytes profile's own size, for a release build"]
fn a_bytes_store_reads_drains_and_indexes_as_its_model_states() {
    let spec = Spec::load_default().unwrap();
    let model = Profile::Bytes.generate(&spec, SEEDS[0]).unwrap();
    let differences = differences(model);
    assert!(differences.is_empty(), "{differences:#?}");
}

/// Every disagreement over `model` given its queue: the reads with the
/// queue pending, the reads once io-pimdir drained it, and the index
/// refreshed against one rebuilt.
pub fn differences(model: Model) -> Vec<String> {
    let spec = Spec::load_default().unwrap();
    let model = queued(model);
    let mut drained = model.clone();
    drained.drain_all();
    let generated = Generated::write(Spec::load_default().unwrap(), model.clone());
    let root = generated.root();

    let mut differences = Vec::new();
    let (_, read) = reader::compare(&spec, &model, &root, |_, _, _, _| None);
    differences.extend(read.into_iter().map(|(_, text)| format!("pending: {text}")));

    let mut owner = PimdirStore::open(&root)
        .unwrap()
        .for_source("local")
        .unwrap();
    owner.drain().unwrap();
    drop(owner);
    let (rule, recorded) = (senders(&drained), drained_senders(&model));
    let (_, read) = reader::compare(&spec, &drained, &root, |expectation, columns, got, want| {
        sender(&rule, &recorded, expectation, columns, got, want)
    });
    differences.extend(
        read.into_iter()
            .filter(|(accepted, _)| accepted.is_none())
            .map(|(_, text)| format!("drained: {text}")),
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
            differences.push(format!(
                "index, {table}: {} rows refreshed, {} rebuilt",
                rows.len(),
                rebuilt.get(table).map_or(0, Vec::len)
            ));
        }
    }
    differences
}

/// The `(mail, contact)` link ids STORAGE §14.2's `sender` rule records
/// over `model`: a mail to every contact whose `email` rows hold one of
/// the mail's `from` addresses (Annex A.6), neither end under a
/// writer-derived key, which no reference names (STORAGE §9).
pub fn senders(model: &Model) -> BTreeSet<(String, String)> {
    let of = |kind: Kind, role: &'static str| {
        model
            .items
            .iter()
            .filter(move |item| model.collections[&item.collection].kind == Some(kind))
            .filter(|item| {
                !["alt:", "dup:", "hash:"]
                    .iter()
                    .any(|prefix| item.link_id.starts_with(prefix))
            })
            .flat_map(move |item| {
                item.addresses
                    .iter()
                    .filter(move |address| address.role == role)
                    .map(move |address| (item.link_id.clone(), address.address.clone()))
            })
            .collect::<BTreeSet<(String, String)>>()
    };
    let contacts = of(Kind::Contact, "email");

    of(Kind::Mail, "from")
        .into_iter()
        .flat_map(|(mail, from)| {
            contacts
                .iter()
                .filter(move |(_, email)| *email == from)
                .map(move |(contact, _)| (mail.clone(), contact.clone()))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// `recorded`, the `sender` references a store may hold, once its owner
/// wrote `model`: a reference goes with the last row of either end, and
/// each one the rule states over `model` joins. A reference is recorded
/// once and never recomputed (STORAGE §14.2), so a card that leaves an
/// address keeps the mail it was the sender of.
pub fn record(recorded: &mut BTreeSet<(String, String)>, model: &Model) {
    let held = |kind: Kind, link_id: &str| {
        model.items.iter().any(|item| {
            item.link_id == link_id && model.collections[&item.collection].kind == Some(kind)
        })
    };
    recorded.retain(|(mail, contact)| held(Kind::Mail, mail) && held(Kind::Contact, contact));
    recorded.extend(senders(model));
}

/// The `sender` references a store of `model` may hold once its owner
/// drained the queue: [`record`] over the model before any row and after
/// each add or update, the rows that change what the rule reads. A drain
/// drops no row of `items`.
pub fn drained_senders(model: &Model) -> BTreeSet<(String, String)> {
    let mut model = model.clone();
    let mut recorded = BTreeSet::new();
    record(&mut recorded, &model);
    let pending: Vec<(i64, bool)> = model
        .queue
        .iter()
        .filter(|row| row.state == RowState::Pending)
        .map(|row| {
            let writes = matches!(row.action, Action::Add { .. } | Action::Update { .. });
            (row.id, writes)
        })
        .collect();
    for (id, writes) in pending {
        model.drain(id);
        if writes {
            record(&mut recorded, &model);
        }
    }
    recorded
}

/// [`SENDER`] where io-pimdir answers every reference the oracle states
/// and more, each more one the store may hold (`recorded`, [`record`]),
/// and, from a mail, every one the rule states over the model (`rule`): an
/// owner applying the rule records what it states (STORAGE §14.2).
pub fn sender(
    rule: &BTreeSet<(String, String)>,
    recorded: &BTreeSet<(String, String)>,
    expectation: &Expectation,
    columns: &[&str],
    got: &[Vec<Value>],
    want: &[Vec<Value>],
) -> Option<&'static str> {
    if !matches!(
        expectation.statement.as_str(),
        "storage/read/references_from" | "storage/read/references_to"
    ) {
        return None;
    }
    let at = |name: &str| columns.iter().position(|column| *column == name);
    let (from, to, role, origin) = (
        at("from_link_id")?,
        at("to_link_id")?,
        at("role")?,
        at("origin")?,
    );
    let text = |value: &Value| match value {
        Value::Text(text) => text.clone(),
        _ => String::new(),
    };

    let extra: Vec<&Vec<Value>> = got.iter().filter(|row| !want.contains(row)).collect();
    let answered: BTreeSet<(String, String)> = extra
        .iter()
        .map(|row| (text(&row[from]), text(&row[to])))
        .collect();
    let ruled = extra.iter().all(|row| {
        text(&row[role]) == "sender"
            && text(&row[origin]) == "auto"
            && recorded.contains(&(text(&row[from]), text(&row[to])))
    });
    let endpoint = expectation
        .params
        .iter()
        .find(|(name, _)| name == "link_id")
        .map(|(_, value)| text(value))?;
    let whole = rule
        .iter()
        .filter(|(mail, _)| *mail == endpoint)
        .all(|pair| answered.contains(pair));
    let kept = want.iter().all(|row| got.contains(row));

    (kept && ruled && whole).then_some(SENDER)
}

/// `model` with a producer's rows over its live mail: a flag change on one
/// item in 7 stating the set it read, a remove on one in 13 and a move into
/// its account's archive on one in 11, each pending. A move out of a
/// collection no source syncs, and one of a bodiless item whose binding
/// is unbased or whose source does not sync the archive, parks (STORAGE
/// §15.3, SYNC §7).
fn queued(mut model: Model) -> Model {
    let mail: Vec<(String, i64, Option<BTreeSet<String>>)> = model
        .items
        .iter()
        .filter(|item| item.state == State::Live)
        .filter(|item| model.collections[&item.collection].kind == Some(Kind::Mail))
        .map(|item| (item.collection.clone(), item.seq, item.flags.clone()))
        .collect();
    for (index, (collection, seq, flags)) in mail.into_iter().enumerate() {
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
