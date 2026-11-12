//! The CLI's verbs over a store of another draft (pimdir STORAGE §4.2, §6):
//! the binary opens a store of its own draft alone, whatever the profile,
//! and refuses any other, leaving it as it was.
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

/// A store holding one queued row, marked as a later draft's, whose
/// `task_summary` carries a column that draft added; answers the row's id.
fn newer_store(dir: &Path) -> i64 {
    let store = PimdirStore::open(dir).unwrap();
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    drop(store);
    let id = PimdirProducer::open(dir, "test")
        .unwrap()
        .enqueue("INBOX", &PimdirAction::Remove { seq: 1 }, None)
        .unwrap();
    let conn = rusqlite::Connection::open(dir.join("pimdir.db")).unwrap();
    conn.execute_batch(
        "UPDATE store_meta SET format_version = 'storage-draft-11';
         ALTER TABLE task_summary ADD COLUMN newer TEXT;",
    )
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

/// A verb that enqueues then drains refuses a store of another draft at the
/// enqueue: nothing queued, the store left as it was.
#[test]
fn a_restore_over_a_store_of_another_draft_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = PimdirCollectionId("INBOX".into());
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    let body = b"Message-ID: <a@example.org>\r\nSubject: a\r\n\r\nText\r\n";
    let hash = PimdirHash("cafebabecafebabecafebabecafebabecafebabecafebabecafe".into());
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
    conn.execute_batch("UPDATE store_meta SET format_version = 'storage-draft-11';")
        .unwrap();
    drop(conn);
    let before = std::fs::read(dir.path().join("pimdir.db")).unwrap();

    let (code, json) = pimdir(
        dir.path(),
        &["--source", "local", "item", "restore", &seq.to_string()],
    );
    assert_ne!(code, 0, "{json}");
    assert!(
        json["error"]
            .as_str()
            .is_some_and(|error| error.contains("storage-draft-11")),
        "{json}"
    );
    assert_eq!(
        std::fs::read(dir.path().join("pimdir.db")).unwrap(),
        before,
        "nothing queued"
    );
}
