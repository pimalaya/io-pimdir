//! A delta's load (pimdir SYNC.md §10, STORAGE.md §14): `Owing` answers
//! the placements it names and every one that may owe the source a push,
//! `list_owing_bindings` and `list_lacking_items` naming them, never the
//! clean placements of the collection; and the system flags are bits of
//! the row, every other flag a row (STORAGE §13).

#![cfg(feature = "client")]

use std::{collections::BTreeSet, path::Path};

use io_pimdir::{
    change::PimdirWriteOp,
    client::{PimdirSourceStore, PimdirStore},
    collection::PimdirCollectionId,
    load::PimdirLoadSelection,
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
};

fn inbox() -> PimdirCollectionId {
    PimdirCollectionId("INBOX".into())
}

/// A bound message at `Meta`, its flags `local` over a base of `base`.
fn placement(handle: &str, local: &[&str], base: &[&str]) -> PimdirPlacement {
    PimdirPlacement {
        sort_key: Default::default(),
        collection: inbox(),
        handle: PimdirHandle(handle.into()),
        link_id: Some(PimdirLinkId(format!("{handle}@example.org"))),
        object: None,
        level: PimdirLevel::Meta,
        summary: None,
        flags: PimdirFlags::from_iter(local.iter().copied()),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags: PimdirFlags::from_iter(base.iter().copied()),
            revision: None,
            object: None,
        }),
        origin: None,
    }
}

fn seeded(dir: &Path, placements: Vec<PimdirPlacement>) -> PimdirSourceStore {
    let mut store = PimdirStore::open(dir).unwrap().for_source("imap");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    let batch = placements.into_iter().map(PimdirWriteOp::UpsertPlacement);
    store.write(batch.collect()).unwrap();
    store
}

fn handles(store: &PimdirSourceStore, selection: &PimdirLoadSelection) -> BTreeSet<String> {
    let loaded = store.load(&inbox(), selection).unwrap();
    loaded.placements.into_iter().map(|p| p.handle.0).collect()
}

#[test]
fn a_delta_loads_what_it_names_and_what_owes_a_push() {
    let dir = tempfile::tempdir().unwrap();
    let mut placements: Vec<PimdirPlacement> = (0..50)
        .map(|n| placement(&format!("c{n}"), &["\\Seen"], &["\\Seen"]))
        .collect();
    placements.push(placement("bit", &["\\Seen", "\\Flagged"], &["\\Seen"]));
    placements.push(placement("row", &["\\Seen", "$label"], &["\\Seen"]));
    let store = seeded(dir.path(), placements);

    let owing = PimdirLoadSelection::Owing(vec![PimdirHandle("c7".into())]);
    let expected: BTreeSet<String> = ["bit", "c7", "row"].map(String::from).into();
    assert_eq!(handles(&store, &owing), expected);
    assert_eq!(handles(&store, &PimdirLoadSelection::All).len(), 52);
}

#[test]
fn a_system_flag_is_a_bit_and_its_lowercase_a_row() {
    let dir = tempfile::tempdir().unwrap();
    let flags = ["\\Seen", "\\seen", "\\Flagged", "$label"];
    let store = seeded(dir.path(), vec![placement("1", &flags, &["\\Seen"])]);

    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    let (marks, base_marks): (i64, i64) = conn
        .query_row(
            "SELECT i.marks, b.base_marks FROM items i JOIN bindings b ON b.item = i.id",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let rows: Vec<String> = conn
        .prepare("SELECT flag FROM placement_flag ORDER BY flag")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!((marks, base_marks), (5, 1), "\\Seen 1 and \\Flagged 4");
    assert_eq!(rows, ["$label", "\\seen"], "every other flag a row");

    let loaded = store.load(&inbox(), &PimdirLoadSelection::All).unwrap();
    assert_eq!(loaded.placements[0].flags, PimdirFlags::from_iter(flags));
}
