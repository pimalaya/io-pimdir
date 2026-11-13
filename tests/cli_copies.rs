//! The CLI's verbs over the rules draft 10 put into text (pimdir STORAGE
//! §7, §8, §15.2, §15.5): `store backup`, `store restore`, `check --deep`,
//! `queue retry`, and an unconfirmed intent as the queue verbs show it.
//!
//! A file of its own, so no test forking the binary shares a process with
//! tests taking the owner lock.

#![cfg(feature = "cli")]

use std::{fs, path::Path, process::Command};

use io_pimdir::{
    client::{PimdirStore, producer::PimdirProducer, reader::PimdirReader},
    codec::PimdirAction,
    placement::PimdirFlags,
    sql,
};
use serde_json::Value;

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

/// A store holding `ids` as messages of its INBOX, no handle left open.
fn populated(dir: &Path, ids: &[&str]) {
    let mut store = PimdirStore::open(dir).unwrap().for_source("local");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    let mut producer = PimdirProducer::open(dir, "test").unwrap();
    for id in ids {
        let body = format!("Message-ID: <{id}>\r\nSubject: {id}\r\n\r\nbody of {id}\r\n");
        producer
            .enqueue_with_body("INBOX", body.as_bytes(), |hash| PimdirAction::Add {
                link_id: None,
                flags: PimdirFlags::default(),
                object: Some(hash),
                supersedes: None,
            })
            .unwrap();
    }
    drop(producer);
    assert_eq!(store.drain().unwrap().applied, ids.len());
}

/// `store backup` copies the store, `store restore` puts the copy back
/// under a new incarnation, asking first when a store is there, and
/// `check --deep` passes on what was restored.
#[test]
fn a_store_is_backed_up_and_restored_under_a_new_incarnation() {
    let dir = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let backup = scratch.path().join("backup");
    let backup_arg = backup.to_str().unwrap();
    populated(dir.path(), &["a@x", "b@x"]);
    let (_, info) = pimdir(dir.path(), &["store", "info"]);
    let incarnation = info["incarnation"].as_i64().unwrap();

    let (code, out) = pimdir(dir.path(), &["store", "backup", backup_arg]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out["objects"], 2);
    assert_eq!(out["incarnation"], Value::Null);
    assert!(backup.join("pimdir.db").is_file());

    let (code, out) = pimdir(dir.path(), &["store", "backup", backup_arg]);
    assert_ne!(code, 0);
    assert_eq!(out["code"], "backup-occupied", "{out}");

    // replacing a store destroys what it held since: confirmed
    let (code, out) = pimdir(dir.path(), &["store", "restore", backup_arg]);
    assert_ne!(code, 0);
    assert_eq!(out["code"], "confirmation-required", "{out}");
    let (_, info) = pimdir(dir.path(), &["store", "info"]);
    assert_eq!(info["incarnation"], incarnation, "left as it was");

    let (code, out) = pimdir(dir.path(), &["store", "restore", backup_arg, "--yes"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out["replaced"], incarnation);
    let restored = out["incarnation"].as_i64().unwrap();
    assert!(restored > incarnation);
    let (_, info) = pimdir(dir.path(), &["store", "info"]);
    assert_eq!(info["incarnation"], restored);
    assert_eq!(info["liveItems"], 2);

    // a directory holding no store takes the backup unasked
    let fresh = scratch.path().join("fresh");
    let (code, out) = pimdir(&fresh, &["store", "restore", backup_arg]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out["replaced"], Value::Null);

    let (code, out) = pimdir(&fresh, &["check", "--deep"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out["deep"]["hashed"], 2);
    assert_eq!(out["deep"]["failing"], Value::Array(Vec::new()));
    let (_, out) = pimdir(&fresh, &["check"]);
    assert_eq!(out["deep"], Value::Null, "only when asked");
}

/// `check` printed a store consistent without hashing a body: `--deep`
/// hashes every one, reports a flipped byte and exits 2, moving nothing.
#[test]
fn a_deep_check_reports_a_flipped_byte() {
    let dir = tempfile::tempdir().unwrap();
    populated(dir.path(), &["a@x", "b@x"]);
    let reader = PimdirReader::open(dir.path()).unwrap();
    let hash = reader.indexed_hashes().unwrap().into_iter().next().unwrap();
    let path = reader
        .blobs()
        .path(&io_pimdir::object::PimdirHash(hash.clone()))
        .unwrap();
    drop(reader);
    let mut bytes = fs::read(&path).unwrap();
    bytes[0] ^= 1;
    fs::write(&path, &bytes).unwrap();

    let (code, out) = pimdir(dir.path(), &["check"]);
    assert_eq!(code, 0, "the plain check hashes nothing: {out}");

    let (code, out) = pimdir(dir.path(), &["check", "--deep"]);
    assert_eq!(code, 2, "{out}");
    assert_eq!(out["deep"]["failing"], serde_json::json!([hash]));
    assert_eq!(out["deep"]["integrity"], Value::Array(Vec::new()));
    assert!(path.exists(), "reported, never moved");
}

/// An unconfirmed intent reads as such in `queue status` and `queue list
/// --parked`; `queue retry` puts a parked row back, asking first for an
/// unconfirmed one, which may be performed twice.
#[test]
fn a_parked_row_is_retried_and_an_unconfirmed_one_shown() {
    let dir = tempfile::tempdir().unwrap();
    populated(dir.path(), &["a@x"]);
    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    let mut ids = Vec::new();
    for _ in 0..2 {
        conn.execute(
            sql::ENQUEUE_ACTION,
            rusqlite::named_params! {
                ":producer": "frontend",
                ":collection": "INBOX",
                ":action": "x-fax-send",
                ":payload": r#"{"v":1}"#,
                ":target_collection": None::<String>,
                ":object_hash": None::<String>,
            },
        )
        .unwrap();
        ids.push(conn.last_insert_rowid());
    }
    drop(conn);
    let mut store = PimdirStore::open(dir.path()).unwrap();
    assert!(store.unconfirm_action(ids[0], "connection lost").unwrap());
    store.fail_action(ids[1], Some("no fax machine")).unwrap();
    drop(store);
    let (sent, failed) = (ids[0].to_string(), ids[1].to_string());

    let (code, out) = pimdir(dir.path(), &["queue", "status", &sent]);
    assert_eq!(code, 0);
    assert_eq!(out["status"], "unconfirmed", "{out}");
    assert_eq!(out["error"], "connection lost");
    let (_, out) = pimdir(dir.path(), &["queue", "list", "--parked"]);
    let rows = out["actions"].as_array().unwrap();
    assert_eq!(rows[0]["unconfirmed"], true, "{out}");
    assert_eq!(rows[0]["error"], "unconfirmed: connection lost");
    assert_eq!(rows[1]["unconfirmed"], false);

    // a failure is retried unasked, an unconfirmed intent only confirmed
    let (code, out) = pimdir(dir.path(), &["queue", "retry", &failed]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out["unconfirmed"], false);
    let (code, out) = pimdir(dir.path(), &["queue", "retry", &sent]);
    assert_ne!(code, 0);
    assert_eq!(out["code"], "confirmation-required", "{out}");
    let (_, out) = pimdir(dir.path(), &["queue", "status", &sent]);
    assert_eq!(out["status"], "unconfirmed", "left parked");

    let (code, out) = pimdir(dir.path(), &["queue", "retry", &sent, "--yes"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out["unconfirmed"], true);
    let (_, out) = pimdir(dir.path(), &["queue", "status", &sent]);
    assert_eq!(out["status"], "pending");
    assert_eq!(out["attempts"], 1, "its attempts kept");

    let (code, out) = pimdir(dir.path(), &["queue", "retry", &sent, "--yes"]);
    assert_ne!(code, 0, "a pending row is not retried");
    assert_eq!(out["code"], "row-not-found", "{out}");
}
