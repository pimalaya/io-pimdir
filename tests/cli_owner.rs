//! The CLI's owner verbs over a store another revision keeps (pimdir
//! STORAGE §6): the binary is never the store's sync engine, so it takes the
//! store as it stands and refuses one it would have to migrate or
//! reconcile, leaving it as it was.
//!
//! A file of its own, so no test forking the binary shares a process with
//! tests taking the owner lock.

#![cfg(feature = "cli")]

use std::{path::Path, process::Command};

use io_pimdir::{
    change::{PimdirDropReason, PimdirWriteOp},
    client::{PimdirStore, producer::PimdirProducer},
    codec::PimdirAction,
    collection::PimdirCollectionId,
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
};
use serde_json::Value;

/// The columns of `table`, read past every role of the crate.
fn columns(dir: &Path, table: &str) -> Vec<String> {
    let conn = rusqlite::Connection::open(dir.join("pimdir.db")).unwrap();
    let mut stmt = conn
        .prepare("SELECT name FROM pragma_table_info(?1)")
        .unwrap();
    stmt.query_map([table], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// A store holding one queued row, whose `task_summary` carries a column a
/// later revision added; answers the row's id.
fn newer_store(dir: &Path) -> i64 {
    let store = PimdirStore::open(dir).unwrap();
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    drop(store);
    let id = PimdirProducer::open(dir, "test")
        .unwrap()
        .enqueue("INBOX", &PimdirAction::Remove { seq: 1 }, None)
        .unwrap();
    let conn = rusqlite::Connection::open(dir.join("pimdir.db")).unwrap();
    conn.execute_batch("ALTER TABLE task_summary ADD COLUMN newer TEXT;")
        .unwrap();
    id
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
fn an_owner_verb_leaves_a_store_of_another_revision_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let id = newer_store(dir.path());
    let before = columns(dir.path(), "task_summary");
    assert!(before.iter().any(|column| column == "newer"));

    let (cancel, _) = pimdir(dir.path(), &["queue", "cancel", &id.to_string(), "--yes"]);
    assert_ne!(cancel, 0, "queue cancel refused");
    assert_eq!(columns(dir.path(), "task_summary"), before, "not rebuilt");

    let (gc, _) = pimdir(dir.path(), &["gc"]);
    assert_ne!(gc, 0, "gc refused");
    assert_eq!(columns(dir.path(), "task_summary"), before, "not rebuilt");
}

/// A verb that enqueues then drains leaves its action queued for the store's
/// sync engine when the store is of another revision: queued, not failed,
/// the store left as it was.
#[test]
fn a_restore_over_a_store_of_another_revision_stays_queued() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = PimdirCollectionId("INBOX".into());
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    let body = b"Message-ID: <a@example.org>\r\nSubject: a\r\n\r\nText\r\n";
    let hash = PimdirHash("cafebabe".into());
    store
        .write(vec![
            PimdirWriteOp::StoreObject {
                object: PimdirObject {
                    hash: hash.clone(),
                    size: body.len(),
                },
                body: Some(body.to_vec()),
            },
            PimdirWriteOp::UpsertPlacement(PimdirPlacement {
                sort_key: Default::default(),
                collection: inbox.clone(),
                handle: PimdirHandle("1".into()),
                link_id: Some(PimdirLinkId("mid:a@example.org".into())),
                object: Some(hash.clone()),
                level: PimdirLevel::Full,
                summary: None,
                flags: PimdirFlags::default(),
                status: PimdirStatus::Clean,
                conflict_revision: None,
                conflict_object: None,
                base: Some(PimdirBase {
                    flags: PimdirFlags::default(),
                    revision: None,
                    object: Some(hash),
                }),
                origin: None,
            }),
        ])
        .unwrap();
    store
        .write(vec![PimdirWriteOp::DropPlacement {
            collection: inbox.clone(),
            handle: PimdirHandle("1".into()),
            reason: PimdirDropReason::Deleted,
        }])
        .unwrap();
    let seq = store.list_retained(inbox, None, 10).unwrap()[0].seq;
    drop(store);
    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch("ALTER TABLE task_summary ADD COLUMN newer TEXT;")
        .unwrap();
    drop(conn);
    let before = columns(dir.path(), "task_summary");

    let (code, json) = pimdir(
        dir.path(),
        &["--source", "local", "item", "restore", &seq.to_string()],
    );
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["status"], "queued", "{json}");
    assert_eq!(columns(dir.path(), "task_summary"), before, "not rebuilt");
}
