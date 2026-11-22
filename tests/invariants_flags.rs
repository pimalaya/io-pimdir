//! The flag columns (STORAGE §13, storage-draft-17), ported from pimdir's
//! retired checks/invariants.sh, whose flag rows became `items.keywords`
//! and `bindings.base_keywords`: an unknown set holds no flag, and a set
//! follows its item through a rename.

#![cfg(feature = "client")]

mod invariants_common;

use io_pimdir::{
    change::PimdirWriteOp, collection::PimdirCollectionId, load::PimdirLoadSelection,
    placement::PimdirFlags,
};

use invariants_common::*;

const AUG: &str = "Sat, 1 Aug 2026 10:00:00 +0000";

/// A store holding `a@x` in INBOX, flagged `\Seen $todo` over a base of
/// `\Seen $done`.
fn flagged(dir: &std::path::Path) -> io_pimdir::client::PimdirSourceStore {
    let mut store = mail_store(dir, "imap");
    let a = message("a@x", "A", AUG, Some("alice@x"));
    let mut placement = held(&store, "INBOX", "1", &a);
    placement.flags = PimdirFlags::from_iter(["\\Seen", "$todo"]);
    placement.base.as_mut().unwrap().flags = PimdirFlags::from_iter(["\\Seen", "$done"]);
    store
        .write(vec![
            body(&store, &a),
            PimdirWriteOp::UpsertPlacement(placement),
        ])
        .unwrap();
    store
}

/// L3826: the schema refuses an item whose flags are unknown holding a
/// system flag bit.
#[test]
fn l3826_an_unknown_set_holds_no_bit() {
    let dir = tempfile::tempdir().unwrap();
    let _store = flagged(dir.path());
    let conn = peek(dir.path());
    let write = conn.execute(
        "UPDATE items SET flags_known = 0, keywords = '' WHERE link_id = 'a@x'",
        [],
    );
    assert!(refused(write, "flags_known = 1 OR"));
    let base = conn.execute(
        "UPDATE bindings SET base_flags_known = 0, base_keywords = ''",
        [],
    );
    assert!(refused(base, "base_flags_known = 1 OR"), "the base alike");
}

/// L351: the schema refuses a keyword on an item whose flags are unknown.
#[test]
fn l351_an_unknown_set_refuses_a_keyword() {
    let dir = tempfile::tempdir().unwrap();
    let _store = flagged(dir.path());
    let conn = peek(dir.path());
    let write = conn.execute(
        "UPDATE items SET flags_known = 0, marks = 0 WHERE link_id = 'a@x'",
        [],
    );
    assert!(refused(write, "flags_known = 1 OR"));
    let base = conn.execute(
        "UPDATE bindings SET base_flags_known = 0, base_marks = 0",
        [],
    );
    assert!(refused(base, "base_flags_known = 1 OR"), "the base alike");
}

/// L356: a set becoming unknown holds nothing: only a write clearing its
/// bits and keywords with it is taken.
#[test]
fn l356_a_set_becoming_unknown_holds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let _store = flagged(dir.path());
    let conn = peek(dir.path());
    let keeping = conn.execute("UPDATE items SET flags_known = 0 WHERE link_id = 'a@x'", []);
    assert!(refused(keeping, "flags_known = 1 OR"));
    conn.execute(
        "UPDATE items SET flags_known = 0, marks = 0, keywords = '' WHERE link_id = 'a@x'",
        [],
    )
    .unwrap();
}

/// L360: a set follows its item through a collection rename, item and
/// base alike.
#[test]
fn l360_a_rename_carries_the_flags() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = flagged(dir.path());
    store.rename_collection("INBOX", "Inbox").unwrap();
    let loaded = store
        .load(
            &PimdirCollectionId("Inbox".into()),
            &PimdirLoadSelection::All,
        )
        .unwrap();
    assert_eq!(loaded.placements.len(), 1);
    let placement = &loaded.placements[0];
    assert_eq!(placement.flags, PimdirFlags::from_iter(["\\Seen", "$todo"]));
    assert_eq!(
        placement.base.as_ref().unwrap().flags,
        PimdirFlags::from_iter(["\\Seen", "$done"])
    );
}
