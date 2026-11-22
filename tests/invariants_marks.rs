//! The owing and lacking marks (STORAGE §4.6) after every write, ported
//! from pimdir's retired checks/invariants.sh: each mark equals its rule,
//! no binding that may owe and no live body a syncing source binds not is
//! left unmarked, and the owing load answers what a full walk finds.

#![cfg(feature = "client")]

mod invariants_common;

use std::{collections::BTreeSet, path::Path};

use io_pimdir::{
    change::PimdirWriteOp,
    client::{PimdirSourceStore, PimdirStore},
    collection::{PimdirCheckpoint, PimdirCollectionId},
    load::PimdirLoadSelection,
    placement::{PimdirFlags, PimdirStatus},
};

use invariants_common::*;

const AUG: &str = "Sat, 1 Aug 2026 10:00:00 +0000";

/// The probe of checks/invariants.sh, its flag rows read as the keyword
/// columns: one line per mark breaking its rule.
const PROBE: &str = "
    SELECT 'owes ' || b.collection || ' ' || b.source || ' ' || b.handle
    FROM bindings b LEFT JOIN binding_owes o ON o.item = b.item AND o.source = b.source
    WHERE b.owes != coalesce(o.owes, 1)
    UNION ALL
    SELECT 'unmarked ' || b.collection || ' ' || b.source || ' ' || b.handle
    FROM bindings b JOIN items i ON i.id = b.item
    WHERE b.owes = 0
      AND (b.conflicted = 1 OR i.conflicted = 1 OR i.deleted = 1
           OR b.base_present = 0
           OR (i.flags_known = 1 AND b.base_flags_known = 1
               AND (i.marks != b.base_marks OR i.keywords != b.base_keywords))
           OR (i.object IS NOT NULL AND i.object IS NOT b.base_object))
    UNION ALL
    SELECT 'lacks ' || i.collection || ' ' || i.link_id
    FROM items i JOIN item_lacks l ON l.item = i.id
    WHERE i.lacks != l.lacks
    UNION ALL
    SELECT 'unlacked ' || i.collection || ' ' || i.link_id || ' ' || s.source
    FROM sources s JOIN items i ON i.collection = s.collection
    WHERE i.lacks = 0 AND i.deleted = 0 AND i.object IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM bindings b WHERE b.item = i.id AND b.source = s.source)";

/// Every mark breaking its rule, empty when all hold.
fn probe(dir: &Path) -> Vec<String> {
    let conn = peek(dir);
    let mut stmt = conn.prepare(PROBE).unwrap();
    stmt.query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The owner, handing the store to `source`.
fn owner(dir: &Path, source: &str) -> PimdirSourceStore {
    PimdirStore::open(dir).unwrap().for_source(source)
}

fn inbox(collection: &str) -> PimdirCollectionId {
    PimdirCollectionId(collection.into())
}

fn checkpoint(collection: &str) -> PimdirWriteOp {
    PimdirWriteOp::SetCheckpoint {
        collection: inbox(collection),
        checkpoint: PimdirCheckpoint(b"c1".to_vec()),
    }
}

/// The handles a full walk finds owing: a status other than clean, no
/// base, flags or a body the base lacks.
fn walked(store: &PimdirSourceStore, collection: &str) -> BTreeSet<String> {
    let loaded = store
        .load(&inbox(collection), &PimdirLoadSelection::All)
        .unwrap();
    loaded
        .placements
        .into_iter()
        .filter(|p| match &p.base {
            None => true,
            Some(base) => {
                p.status != PimdirStatus::Clean
                    || base.flags != p.flags
                    || (p.object.is_some() && p.object != base.object)
            }
        })
        .map(|p| p.handle.0)
        .collect()
}

/// The handles the mark-seeking load answers.
fn owing(store: &PimdirSourceStore, collection: &str) -> BTreeSet<String> {
    let loaded = store
        .load(&inbox(collection), &PimdirLoadSelection::Owing(Vec::new()))
        .unwrap();
    loaded.placements.into_iter().map(|p| p.handle.0).collect()
}

fn lacking(dir: &Path) -> BTreeSet<String> {
    let conn = peek(dir);
    let mut stmt = conn
        .prepare("SELECT link_id FROM items WHERE lacks = 1")
        .unwrap();
    stmt.query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn set(links: &[&str]) -> BTreeSet<String> {
    links.iter().map(|link| link.to_string()).collect()
}

/// G1, L3955: after every write the marks equal their rules, nothing that
/// may owe or lack is unmarked, and the owing load answers what a full
/// walk of the placements finds.
#[test]
fn g1_l3955_the_marks_hold_after_every_write() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let mail = |name: &str| message(&format!("{name}@x"), name, AUG, Some("alice@x"));
    let (a, b, c, d) = (mail("a"), mail("b"), mail("c"), mail("d"));
    let mut seen_owes = false;
    let mut check = |store: &PimdirSourceStore, step: &str, collection: &str| {
        let drift = probe(dir);
        assert!(drift.is_empty(), "after {step}: {drift:#?}");
        assert_eq!(
            owing(store, collection),
            walked(store, collection),
            "after {step}: the marks answer what the walk finds"
        );
        seen_owes |= int(&peek(dir), "SELECT count(*) FROM bindings WHERE owes = 1") > 0;
    };

    let mut imap = mail_store(dir, "imap");
    imap.write(vec![
        body(&imap, &a),
        body(&imap, &b),
        body(&imap, &c),
        PimdirWriteOp::UpsertPlacement(held(&imap, "INBOX", "1", &a)),
        PimdirWriteOp::UpsertPlacement(held(&imap, "INBOX", "2", &b)),
        PimdirWriteOp::UpsertPlacement(held(&imap, "INBOX", "3", &c)),
        checkpoint("INBOX"),
    ])
    .unwrap();
    check(&imap, "three bodies bound", "INBOX");

    let mut placement = held(&imap, "INBOX", "1", &a);
    placement.flags = PimdirFlags::from_iter(["\\Seen"]);
    imap.write(vec![PimdirWriteOp::UpsertPlacement(placement.clone())])
        .unwrap();
    check(&imap, "a flag bit on the item", "INBOX");

    placement.flags = PimdirFlags::from_iter(["\\Seen", "$todo"]);
    imap.write(vec![PimdirWriteOp::UpsertPlacement(placement.clone())])
        .unwrap();
    check(&imap, "a keyword gained by the item", "INBOX");

    placement.base.as_mut().unwrap().flags = PimdirFlags::from_iter(["\\Seen", "$todo"]);
    imap.write(vec![PimdirWriteOp::UpsertPlacement(placement.clone())])
        .unwrap();
    check(&imap, "the base gaining both", "INBOX");

    placement.flags = PimdirFlags::from_iter(["\\Seen"]);
    imap.write(vec![PimdirWriteOp::UpsertPlacement(placement.clone())])
        .unwrap();
    check(&imap, "a keyword lost by the item", "INBOX");

    let theirs = mail("theirs");
    let mut conflicted = held(&imap, "INBOX", "2", &b);
    conflicted.status = PimdirStatus::Conflict;
    conflicted.conflict_revision = Some("r2".into());
    conflicted.conflict_object = Some(imap.hash(&theirs));
    imap.write(vec![
        body(&imap, &theirs),
        PimdirWriteOp::UpsertPlacement(conflicted),
    ])
    .unwrap();
    check(&imap, "a binding conflict", "INBOX");

    imap.write(vec![PimdirWriteOp::UpsertPlacement(held(
        &imap, "INBOX", "2", &b,
    ))])
    .unwrap();
    check(&imap, "the conflict settled", "INBOX");

    let mut tombstone = held(&imap, "INBOX", "3", &c);
    tombstone.status = PimdirStatus::Tombstone;
    imap.write(vec![PimdirWriteOp::UpsertPlacement(tombstone)])
        .unwrap();
    check(&imap, "a tombstone", "INBOX");

    imap.write(vec![deleted("INBOX", "3")]).unwrap();
    check(&imap, "the delete confirmed", "INBOX");

    imap.write(vec![PimdirWriteOp::UpsertPlacement(held(
        &imap, "INBOX", "3", &c,
    ))])
    .unwrap();
    check(&imap, "a revive", "INBOX");

    imap.write(vec![PimdirWriteOp::UpsertPlacement(meta("INBOX", "4", &d))])
        .unwrap();
    check(&imap, "a bodiless item", "INBOX");

    imap.write(vec![
        body(&imap, &d),
        PimdirWriteOp::UpsertPlacement(held(&imap, "INBOX", "4", &d)),
    ])
    .unwrap();
    check(&imap, "its body arriving", "INBOX");
    drop(imap);

    let mut graph = owner(dir, "graph");
    graph.write(vec![checkpoint("INBOX")]).unwrap();
    check(&graph, "a source joining", "INBOX");
    graph
        .write(vec![PimdirWriteOp::UpsertPlacement(held(
            &graph, "INBOX", "g1", &a,
        ))])
        .unwrap();
    check(&graph, "a binding gained", "INBOX");
    graph.write(vec![deleted("INBOX", "g1")]).unwrap();
    check(&graph, "a binding lost", "INBOX");

    graph.rename_collection("INBOX", "Inbox").unwrap();
    check(&graph, "a rename", "Inbox");
    peek(dir)
        .execute("DELETE FROM sources WHERE source = 'graph'", [])
        .unwrap();
    check(&graph, "a source leaving", "Inbox");
    drop(graph);

    assert!(seen_owes, "the probe saw an owed binding");
}

/// L3986, L3886: a source joining a collection marks every live item
/// holding a body it binds not, and the marks go once it binds them all.
#[test]
fn l3986_l3886_a_source_joining_marks_what_it_binds_not() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let mail = |name: &str| message(&format!("{name}@x"), name, AUG, Some("alice@x"));
    let (a, b, d) = (mail("a"), mail("b"), mail("d"));
    let mut imap = mail_store(dir, "imap");
    imap.write(vec![
        body(&imap, &a),
        body(&imap, &b),
        PimdirWriteOp::UpsertPlacement(held(&imap, "INBOX", "1", &a)),
        PimdirWriteOp::UpsertPlacement(held(&imap, "INBOX", "2", &b)),
        PimdirWriteOp::UpsertPlacement(meta("INBOX", "4", &d)),
        checkpoint("INBOX"),
    ])
    .unwrap();
    assert!(lacking(dir).is_empty(), "imap binds them all");
    drop(imap);

    let mut graph = owner(dir, "graph");
    graph.write(vec![checkpoint("INBOX")]).unwrap();
    assert_eq!(
        lacking(dir),
        set(&["a@x", "b@x"]),
        "L3986: the bodies graph binds not, never the bodiless"
    );
    graph
        .write(vec![
            PimdirWriteOp::UpsertPlacement(held(&graph, "INBOX", "g1", &a)),
            PimdirWriteOp::UpsertPlacement(held(&graph, "INBOX", "g2", &b)),
        ])
        .unwrap();
    assert!(lacking(dir).is_empty(), "L3886: nothing once it binds all");
    assert!(probe(dir).is_empty());
}

/// L4098: a property write derives no push and moves nothing a load
/// compares.
#[test]
fn l4098_a_property_moves_no_mark_and_no_load() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let a = message("a@x", "A", AUG, Some("alice@x"));
    let b = message("b@x", "B", AUG, Some("alice@x"));
    let mut imap = mail_store(dir, "imap");
    let mut owed = held(&imap, "INBOX", "2", &b);
    owed.flags = PimdirFlags::from_iter(["\\Flagged"]);
    imap.write(vec![
        body(&imap, &a),
        body(&imap, &b),
        PimdirWriteOp::UpsertPlacement(held(&imap, "INBOX", "1", &a)),
        PimdirWriteOp::UpsertPlacement(owed),
        checkpoint("INBOX"),
    ])
    .unwrap();
    let marks = "SELECT (SELECT group_concat(handle || owes) FROM bindings) \
                 || '|' || (SELECT group_concat(link_id || lacks) FROM items)";
    let loads = |store: &PimdirSourceStore| {
        (
            store
                .load(&inbox("INBOX"), &PimdirLoadSelection::All)
                .unwrap()
                .placements,
            owing(store, "INBOX"),
        )
    };
    let conn = peek(dir);
    let before: String = conn.query_row(marks, [], |row| row.get(0)).unwrap();
    let loaded = loads(&imap);
    assert_eq!(loaded.1, set(&["2"]), "one owed binding to compare");

    let seq = imap.seq_for_link("INBOX", "a@x").unwrap().unwrap();
    assert!(imap.add_property(seq, "app/label", "work").unwrap());

    let after: String = conn.query_row(marks, [], |row| row.get(0)).unwrap();
    assert_eq!(after, before, "no mark moved");
    assert_eq!(loads(&imap), loaded, "no load moved");
}
