//! # The overlay against the model
//!
//! A reader built [`with_pending`](PimdirReader::with_pending) over a
//! generated store holding queued rows of every producer, against
//! `Model::overlay`, the generator's reading of STORAGE §15.4: each
//! collection walked page by page, every item it shows compared, and each
//! difference sorted under the finding it stands for, so the known ones
//! narrow the test by name and anything else fails it.

use std::collections::{BTreeMap, BTreeSet};

use io_pimdir::client::reader::{PimdirItem, PimdirReader};
use pimdir_generator::{
    model::{Item, Model},
    queue::{Action, RowState},
};
use rusqlite::types::Value;

use crate::common::{Generated, SEEDS, item_cells, table_cells};

/// Finding G4 (implementation, SHOULD): a pending `add` is not shown in
/// the collection it creates into, `pending_creates` reporting it apart,
/// where STORAGE §15.4 folds a row as the owner would apply it.
const G4: &str = "G4: a pending add is not overlaid";

/// Finding G5 (spec minor, reads two ways): a `move` or `copy` queued on
/// another collection shows in its target in io-pimdir, which folds the
/// whole queue, and not in the generator, which folds the target's own
/// `load_pending_actions` as STORAGE §15.4 names them.
const G5: &str = "G5: an arrival from another collection's queue";

/// The page the overlay is walked at, small so the fold crosses pages.
const PAGE: usize = 3;

#[test]
fn an_overlaid_read_shows_what_the_drain_would_leave() {
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
#[ignore = "G4 and G5: the overlay findings of the generated suite"]
fn an_overlaid_read_shows_what_the_drain_would_leave_known_included() {
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

/// Every difference between io-pimdir's overlay and the model's at
/// `seed`, each with the known finding it stands under.
fn differences(seed: u64) -> Vec<(Option<&'static str>, String)> {
    let generated = Generated::structure(seed);
    let model = &generated.model;
    let reader = PimdirReader::open(generated.root()).unwrap().with_pending();

    let mut differences = Vec::new();
    for (id, collection) in &model.collections {
        let shown = walk(&reader, id);
        let count = reader.count_items(id).unwrap() as usize;
        if count != shown.len() {
            let difference = format!("{id}: counts {count}, lists {}", shown.len());
            differences.push((None, difference));
        }
        for (seq, item) in &shown {
            let got = reader.get_item(id, *seq).unwrap();
            if got.as_ref() != Some(item) {
                let difference = format!("{id} seq {seq}: gets {got:?}, lists {item:?}");
                differences.push((None, difference));
            }
        }

        let want: BTreeMap<i64, Item> = model
            .overlay(id)
            .into_iter()
            .map(|item| (item.seq, item))
            .collect();
        let seqs: BTreeSet<i64> = want.keys().chain(shown.keys()).copied().collect();
        for seq in seqs {
            let committed = model.live(id).any(|item| item.seq == seq);
            let rows = Rows {
                model,
                collection: id,
                seq,
            };
            let (known, what) = match (want.get(&seq), shown.get(&seq)) {
                (Some(want), None) if !committed => {
                    (rows.added(&want.link_id).then_some(G4), "not shown")
                }
                (Some(_), None) => (None, "folded out"),
                (None, Some(_)) if !committed => (rows.arrives().then_some(G5), "shown"),
                (None, Some(_)) => (None, "kept"),
                (Some(want), Some(got)) => {
                    let table = collection.kind.map(|_| want.summary.table.stem());
                    let (got_cells, want_cells) = (item_cells(got), model_cells(want));
                    let summary = table.is_some_and(|table| {
                        table_cells(table, got.summary.as_ref()).1 != summary_cells(want, table)
                    });
                    let sorted = got_cells[4] != want_cells[4];
                    let restated = [1, 2, 3, 5]
                        .into_iter()
                        .any(|at| got_cells[at] != want_cells[at]);
                    if !restated && !summary && !sorted {
                        continue;
                    }
                    (None, "restated")
                }
                (None, None) => continue,
            };
            let difference = format!(
                "{id} seq {seq} {what}: io-pimdir {:?}, the model {:?}",
                shown.get(&seq).map(item_cells),
                want.get(&seq).map(model_cells),
            );
            differences.push((known, difference));
        }
    }

    differences
}

/// A collection's overlaid listing walked to its end by keyset pages, each
/// item once, its summary joined.
fn walk(reader: &PimdirReader, collection: &str) -> BTreeMap<i64, PimdirItem> {
    let mut shown = BTreeMap::new();
    let mut after: Option<(String, i64)> = None;
    loop {
        let cursor = after.as_ref().map(|(key, seq)| (key.as_str(), *seq));
        let page = reader.list_summaries(collection, cursor, PAGE).unwrap();
        let Some(last) = page.last() else {
            break;
        };
        after = Some((last.sort_key.clone(), last.seq));
        for item in page {
            let seq = item.seq;
            assert!(
                shown.insert(seq, item).is_none(),
                "{collection}: seq {seq} listed twice"
            );
        }
    }
    shown
}

/// The pending rows that bring one item into a collection or create it
/// there.
struct Rows<'a> {
    model: &'a Model,
    collection: &'a str,
    seq: i64,
}

impl Rows<'_> {
    /// Whether a pending `add` queued on the collection creates the item of
    /// key `link_id`.
    fn added(&self, link_id: &str) -> bool {
        self.model.queue.iter().any(|row| {
            row.state == RowState::Pending
                && row.collection == self.collection
                && matches!(&row.action, Action::Add { link_id: added, .. } if added == link_id)
        })
    }

    /// Whether a `move` or `copy` queued on another collection brings the
    /// item in.
    fn arrives(&self) -> bool {
        self.model.queue.iter().any(|row| {
            row.state == RowState::Pending
                && row.collection != self.collection
                && match &row.action {
                    Action::Move { seq, to } | Action::Copy { seq, to } => {
                        *seq == self.seq && to == self.collection
                    }
                    _ => false,
                }
        })
    }
}

/// A model item in the six item columns.
fn model_cells(item: &Item) -> Vec<Value> {
    let text = |value: Option<&str>| value.map_or(Value::Null, |value| Value::Text(value.into()));
    vec![
        Value::Integer(item.seq),
        Value::Text(item.link_id.clone()),
        item.flags.as_ref().map_or(Value::Null, |flags| {
            Value::Text(flags.iter().cloned().collect::<Vec<_>>().join(" "))
        }),
        text(item.object.as_ref().map(|name| name.0.as_str())),
        Value::Text(item.sort_key.clone()),
        Value::Integer(item.level),
    ]
}

/// A model item's summary in the columns of `table`, `NULL` where its row
/// is of another.
fn summary_cells(item: &Item, table: &str) -> Vec<Value> {
    let (columns, _) = table_cells(table, None);
    columns
        .iter()
        .map(|column| match item.summary.table.stem() == table {
            true => item.summary.get(column),
            false => Value::Null,
        })
        .collect()
}
