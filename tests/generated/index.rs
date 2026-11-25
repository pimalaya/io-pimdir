//! # The index refreshed against the index rebuilt
//!
//! A generated store taken through a run of owner writes chosen from its
//! model (the drain, purges, a restated sort key, a renamed and a deleted
//! collection, released bodies, an eviction, an erase, the collector),
//! its index refreshed after each by the change feed in small batches
//! (SEARCH §4) and compared, table by table and token by token, with the
//! index a copy of the store builds from nothing at that point. Rows are
//! keyed by what they index, never by the row ids the two orders draw.

use std::{
    collections::BTreeMap,
    fs::{self, DirBuilder},
    os::unix::fs::DirBuilderExt,
    path::Path,
};

use io_pimdir::client::{
    PimdirError, PimdirSourceStore, PimdirStore,
    search::{PimdirIndex, PimdirIndexRefreshOptions},
};
use pimdir_generator::{
    model::{Kind, Model, State},
    name::ObjectName,
    profile::Profile,
    queue::RowState,
    spec::Spec,
};
use rusqlite::{Connection, types::Value};

use crate::common::{Generated, SEEDS};

/// Finding G8 (implementation, io-pimdir through ical-rs 0.7.1, high): a
/// body whose `VTIMEZONE` observance repeats `FREQ=SECONDLY` from 1970
/// (vectors/fixtures/event-zoned-hostile.ics) makes the refresh walk every
/// onset to the year after the instant it resolves (`IcalTz::transitions`),
/// about 1.8e9 of them: the indexer aborts on memory, which a remote
/// calendar causes. Indexing it would take the test process down with it,
/// so the test holds such a body as none, the item listed from its summary.
const G8: &str = "G8: a hostile zone rule exhausts the indexer's memory";

/// The instant every refresh draws its horizon around, Unix seconds.
pub const NOW: i64 = 1_790_000_000;

/// What a refresh folds per transaction, small so a pass is cut often.
pub const BATCH: usize = 3;

/// The rows of an index by what they index, per table.
pub type Dump = BTreeMap<&'static str, Vec<Vec<Value>>>;

#[test]
fn an_index_refreshed_after_each_owner_write_is_the_one_a_rebuild_writes() {
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

/// Every table a refreshed index at `seed` holds otherwise than the index
/// rebuilt at the same write, each with the known finding it stands under.
fn differences(seed: u64) -> Vec<(Option<&'static str>, String)> {
    let spec = Spec::load_default().unwrap();
    let model = Profile::Structure.generate(&spec, seed).unwrap();
    differences_of(tamed(model))
}

/// The same over the store of `model`, a model [`tamed`] holds.
pub fn differences_of(model: Model) -> Vec<(Option<&'static str>, String)> {
    let generated = Generated::write(Spec::load_default().unwrap(), model);
    let root = generated.root();
    let mut drained = generated.model.clone();
    drained.drain_all();

    let options = PimdirIndexRefreshOptions {
        batch: BATCH,
        now: Some(NOW),
        ..Default::default()
    };
    let mut index = PimdirIndex::open(&root).unwrap();
    let mut owner = PimdirStore::open(&root)
        .unwrap()
        .for_source("local")
        .unwrap();

    let mut differences = Vec::new();
    for (step, write) in writes(&drained).into_iter().enumerate() {
        let label = write.label();
        assert!(write.apply(&mut owner).unwrap(), "{label}");
        index.refresh(&options).unwrap();

        let refreshed = dump(&root);
        let rebuilt = rebuilt(&root, &options);
        for (table, rows) in &refreshed {
            let built = &rebuilt[table];
            if built == rows {
                continue;
            }
            let only = |left: &Vec<Vec<Value>>, right: &Vec<Vec<Value>>| {
                left.iter()
                    .filter(|row| !right.contains(row))
                    .cloned()
                    .collect::<Vec<_>>()
            };
            let difference = format!(
                "step {step} ({label}), {table}: refreshed alone {:?}, rebuilt alone {:?}",
                only(rows, built),
                only(built, rows),
            );
            differences.push((None, difference));
        }
    }
    differences
}

/// One owner write of the run.
pub enum Write {
    Nothing,
    Drain,
    Purge { collection: String, seq: i64 },
    SortKey { collection: String, link_id: String },
    Name { collection: String },
    Rename { collection: String },
    Release { collection: String },
    Evict { collection: String, before: String },
    Erase { collection: String, seq: i64 },
    Delete { collection: String },
    Collect,
}

impl Write {
    /// What the write is, for a failure to name.
    pub fn label(&self) -> String {
        match self {
            Self::Nothing => String::from("as written"),
            Self::Drain => String::from("drain"),
            Self::Purge { collection, seq } => format!("purge {collection} {seq}"),
            Self::SortKey {
                collection,
                link_id,
            } => format!("sort key {collection} {link_id}"),
            Self::Name { collection } => format!("name {collection}"),
            Self::Rename { collection } => format!("rename {collection}"),
            Self::Release { collection } => format!("release {collection}"),
            Self::Evict { collection, before } => format!("evict {collection} {before}"),
            Self::Erase { collection, seq } => format!("erase {collection} {seq}"),
            Self::Delete { collection } => format!("delete {collection}"),
            Self::Collect => String::from("collect"),
        }
    }

    /// Runs the write as `owner`, answering whether it found what it
    /// names: the purge its item, the delete its collection.
    pub fn apply(&self, owner: &mut PimdirSourceStore) -> Result<bool, PimdirError> {
        match self {
            Self::Nothing => {}
            Self::Drain => {
                owner.drain()?;
            }
            Self::Purge { collection, seq } => return owner.purge(collection, *seq),
            Self::SortKey {
                collection,
                link_id,
            } => owner.set_sort_key(collection, link_id, "2001-01-01T00:00:00Z")?,
            Self::Name { collection } => owner.set_collection_name(collection, "Renamed")?,
            Self::Rename { collection } => {
                owner.rename_collection(collection, &format!("{collection} renamed"))?
            }
            Self::Release { collection } => {
                owner.release_before(&[collection], None)?;
            }
            Self::Evict { collection, before } => {
                owner.evict_before(collection, before)?;
            }
            Self::Erase { collection, seq } => {
                owner.erase(collection, *seq)?;
            }
            Self::Delete { collection } => return owner.delete_collection(collection),
            Self::Collect => {
                owner.collect_garbage()?;
            }
        }
        Ok(true)
    }
}

/// The run of writes the drained model gives, each target drawn from it.
pub fn writes(drained: &Model) -> Vec<Write> {
    let mut writes = vec![Write::Nothing, Write::Drain];

    for item in &drained.items {
        if matches!(item.state, State::Retained { .. }) {
            writes.push(Write::Purge {
                collection: item.collection.clone(),
                seq: item.seq,
            });
        }
    }

    let live = |collection: &str| drained.live(collection).next().cloned();
    writes.push(Write::Name {
        collection: String::from("work/INBOX"),
    });
    writes.push(Write::Rename {
        collection: String::from("work/Archive"),
    });
    writes.push(Write::Release {
        collection: String::from("work/INBOX"),
    });

    let mut dates: Vec<&str> = drained
        .live("home/INBOX")
        .map(|item| item.sort_key.as_str())
        .filter(|key| !key.is_empty())
        .collect();
    dates.sort_unstable();
    if let Some(middle) = dates.get(dates.len() / 2) {
        writes.push(Write::Evict {
            collection: String::from("home/INBOX"),
            before: String::from(*middle),
        });
    }

    if let Some(item) = live("work/calendar") {
        writes.push(Write::Erase {
            collection: item.collection,
            seq: item.seq,
        });
    }

    let queued: Vec<&str> = drained
        .queue
        .iter()
        .filter(|row| !matches!(row.state, RowState::Applied(_)))
        .flat_map(|row| [Some(row.collection.as_str()), row.action.target()])
        .flatten()
        .collect();
    for (id, collection) in &drained.collections {
        if collection.kind == Some(Kind::Mail) || queued.contains(&id.as_str()) {
            continue;
        }
        writes.push(Write::Delete {
            collection: id.clone(),
        });
    }
    writes.push(Write::Delete {
        collection: String::from("work/Empty"),
    });
    writes.push(Write::Collect);

    if let Some(item) = live("work/INBOX") {
        writes.push(Write::SortKey {
            collection: item.collection,
            link_id: item.link_id,
        });
    }

    writes
}

/// The index of a copy of the store at `root`, built from nothing and
/// refreshed with `options`.
pub fn rebuilt(root: &Path, options: &PimdirIndexRefreshOptions) -> Dump {
    let copy = tempfile::tempdir().unwrap();
    let store = copy.path().join("store");
    copy_store(root, &store, false);
    let mut index = PimdirIndex::open(&store).unwrap();
    index.refresh(options).unwrap();
    drop(index);
    dump(&store)
}

/// Copies the store at `from` into `to`, private as the store is, its
/// index files with it when `index` holds.
pub fn copy_store(from: &Path, to: &Path, index: bool) {
    DirBuilder::new().mode(0o700).create(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if !index && name.to_string_lossy().starts_with("index.") {
            continue;
        }
        let target = to.join(&name);
        match entry.file_type().unwrap().is_dir() {
            true => copy_store(&entry.path(), &target, index),
            false => {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }
}

/// Every row of the index at `root` keyed by what it indexes: the bodies a
/// placement names by hash, those none names apart, the placements by
/// collection and `seq`, each body by its hash (`gone` for an id naming
/// no row, ids differing between two indexes), and the full-text tables
/// as the tokens each document holds, read through `fts5vocab`.
pub fn dump(root: &Path) -> Dump {
    let conn = Connection::open(root.join("index.db")).unwrap();
    conn.execute_batch(
        "CREATE VIRTUAL TABLE temp.object_vocab USING fts5vocab(main, object_text, instance);
         CREATE VIRTUAL TABLE temp.summary_vocab USING fts5vocab(main, summary_text, instance);
         CREATE TEMP VIEW placed AS SELECT id, hash, status, attachment, tel, adr, bday, photo \
         FROM object WHERE id IN (SELECT object FROM placement WHERE object IS NOT NULL);",
    )
    .unwrap();

    let tables: [(&str, &str); 11] = [
        (
            "unplaced",
            "SELECT hash, status FROM object WHERE id NOT IN (SELECT id FROM temp.placed) \
             ORDER BY hash",
        ),
        (
            "index_meta",
            "SELECT format, format_version, shape, version, tokenizer, store_id, \
             store_incarnation, store_change, store_purges, horizon_start, horizon_end, \
             tzdb_version, floating_zone, extraction, extractor FROM index_meta",
        ),
        (
            "object",
            "SELECT hash, status, attachment, tel, adr, bday, photo FROM temp.placed ORDER BY hash",
        ),
        (
            "object_category",
            "SELECT o.hash, c.value FROM object_category c JOIN temp.placed o ON o.id = c.object \
             ORDER BY 1, 2",
        ),
        (
            "placement",
            "SELECT p.collection, p.seq, p.link_id, \
             coalesce(o.hash, CASE WHEN p.object IS NOT NULL THEN 'gone' END), p.cut \
             FROM placement p LEFT JOIN object o ON o.id = p.object ORDER BY 1, 2",
        ),
        (
            "missing",
            "SELECT collection, seq FROM missing ORDER BY 1, 2",
        ),
        (
            "flag",
            "SELECT flag, collection, seq FROM flag ORDER BY 1, 2, 3",
        ),
        (
            "occurrence",
            "SELECT collection, seq, start, \"end\" FROM occurrence ORDER BY 1, 2, 3, 4",
        ),
        (
            "message",
            "SELECT link_id, thread, parent FROM message ORDER BY link_id",
        ),
        (
            "object_text",
            "SELECT coalesce(o.hash, 'rowid ' || v.doc), v.term, v.col, v.\"offset\" \
             FROM temp.object_vocab v LEFT JOIN object o ON o.id = v.doc \
             WHERE o.id IS NULL OR o.id IN (SELECT id FROM temp.placed) ORDER BY 1, 2, 3, 4",
        ),
        (
            "summary_text",
            "SELECT coalesce(p.collection || ' ' || p.seq, 'rowid ' || v.doc), v.term, v.col, \
             v.\"offset\" FROM temp.summary_vocab v LEFT JOIN placement p ON p.id = v.doc \
             ORDER BY 1, 2, 3, 4",
        ),
    ];

    tables
        .into_iter()
        .map(|(table, sql)| {
            let mut statement = conn.prepare(sql).unwrap();
            let width = statement.column_count();
            let rows = statement
                .query_map([], |row| {
                    (0..width).map(|at| row.get::<_, Value>(at)).collect()
                })
                .unwrap()
                .map(Result::unwrap)
                .collect();
            (table, rows)
        })
        .collect()
}

/// The model with every body G8 stands under held as none, its item kept
/// at `Meta` and listed from its summary, its seq unmoved.
pub fn tamed(mut model: Model) -> Model {
    let hostile: Vec<ObjectName> = model
        .objects
        .iter()
        .filter(|(_, bytes)| {
            let text = String::from_utf8_lossy(bytes);
            text.contains("BEGIN:VTIMEZONE") && text.contains("FREQ=SECONDLY")
        })
        .map(|(name, _)| name.clone())
        .collect();
    assert!(!hostile.is_empty(), "{G8}: the structure profile files it");

    let held = |name: &mut Option<ObjectName>| {
        if name.as_ref().is_some_and(|name| hostile.contains(name)) {
            *name = None;
        }
    };
    for item in &mut model.items {
        if item
            .object
            .as_ref()
            .is_some_and(|name| hostile.contains(name))
        {
            item.level = 1;
        }
        held(&mut item.object);
        held(&mut item.conflict_object);
        for binding in &mut item.bindings {
            held(&mut binding.base_object);
            held(&mut binding.shared_object);
            held(&mut binding.conflict_object);
        }
    }
    for name in &hostile {
        model.objects.remove(name);
    }
    model
}
