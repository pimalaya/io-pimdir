//! The CLI's `property` verbs (pimdir STORAGE §4.3, §15.3): a change is
//! staged as a `set-property` and applied when the owner role is free, and
//! the listing reads what the store holds.
//!
//! A file of its own, so no test forking the binary shares a process with
//! tests taking the owner lock.

#![cfg(feature = "cli")]

use std::{path::Path, process::Command};

use io_pimdir::{
    change::PimdirWriteOp,
    client::PimdirStore,
    collection::PimdirCollectionId,
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
};
use serde_json::{Value, json};

const BODY: &str = "cafebabecafebabecafebabecafebabecafebabecafebabecafe";

/// A store holding one message in INBOX, bound by `imap`; answers its seq.
fn store(dir: &Path) -> i64 {
    let mut store = PimdirStore::open(dir).unwrap().for_source("imap").unwrap();
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    store
        .write(vec![
            PimdirWriteOp::StoreObject {
                object: PimdirObject {
                    hash: PimdirHash(BODY.into()),
                    size: 4,
                },
                body: Some(b"body".to_vec()),
            },
            PimdirWriteOp::UpsertPlacement(PimdirPlacement {
                sort_key: Default::default(),
                collection: PimdirCollectionId("INBOX".into()),
                handle: PimdirHandle("1".into()),
                link_id: Some(PimdirLinkId("m".into())),
                object: Some(PimdirHash(BODY.into())),
                level: PimdirLevel::Full,
                summary: None,
                flags: PimdirFlags::default(),
                status: PimdirStatus::Clean,
                conflict_revision: None,
                conflict_object: None,
                base: Some(PimdirBase {
                    flags: PimdirFlags::default(),
                    revision: None,
                    object: Some(PimdirHash(BODY.into())),
                }),
                origin: None,
                server_object_id: None,
            }),
        ])
        .unwrap();
    store.seq_for_link("INBOX", "m").unwrap().unwrap()
}

/// Runs the binary over the store with `--json`, answering its exit code
/// and what it printed.
fn pimdir(dir: &Path, args: &[&str]) -> (i32, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_pimdir"))
        .arg("--store")
        .arg(dir)
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    let json = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    (output.status.code().unwrap_or(-1), json)
}

#[test]
fn a_label_is_added_listed_and_removed_through_the_queue() {
    let dir = tempfile::tempdir().unwrap();
    let seq = store(dir.path()).to_string();

    let (code, added) = pimdir(
        dir.path(),
        &["property", "add", &seq, "pimdir/label", "Receipts", "Tax"],
    );
    assert_eq!(code, 0, "{added}");
    assert_eq!(added["collection"], "INBOX");
    assert_eq!(added["queued"], false, "applied at once");

    let (code, _) = pimdir(
        dir.path(),
        &["property", "add", &seq, "pimdir/label", "RECEIPTS"],
    );
    assert_eq!(code, 0);
    let (code, listed) = pimdir(dir.path(), &["property", "list", &seq]);
    assert_eq!(code, 0);
    assert_eq!(
        listed["properties"],
        json!([
            { "key": "pimdir/label", "value": "Receipts" },
            { "key": "pimdir/label", "value": "Tax" },
        ]),
        "a label compares folded"
    );

    let (code, _) = pimdir(
        dir.path(),
        &["property", "remove", &seq, "pimdir/label", "tax"],
    );
    assert_eq!(code, 0);
    let (_, listed) = pimdir(dir.path(), &["property", "list", &seq]);
    assert_eq!(listed["properties"].as_array().unwrap().len(), 1);
}

#[test]
fn a_key_pimdir_keeps_is_refused_and_nothing_is_queued() {
    let dir = tempfile::tempdir().unwrap();
    let seq = store(dir.path()).to_string();

    let (code, refused) = pimdir(
        dir.path(),
        &["property", "add", &seq, "pimdir/session-key", "k"],
    );
    assert_ne!(code, 0);
    assert_eq!(refused["code"], "action-malformed", "{refused}");
    let (_, queue) = pimdir(dir.path(), &["queue", "list"]);
    assert_eq!(queue["actions"], json!([]), "{queue}");
}
