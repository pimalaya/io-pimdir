//! The CLI's JSON contract, as pimalaya-cli's output and error specs set
//! it for every Pimalaya binary, run as the binary over a temporary store
//! with `--json`: a body travels as data, a listing resumes at the
//! `nextPage` it printed, an absent value prints `null`, and a check
//! leaving a problem prints its result and exits 2.
//!
//! A file of its own, so no test forking the binary shares a process with
//! tests taking the owner lock; the tests here take [`SERIAL`] for the
//! same reason.

#![cfg(feature = "cli")]

use std::{
    fs,
    path::Path,
    process::Command,
    sync::{Mutex, PoisonError},
};

use io_pimdir::{
    change::PimdirWriteOp,
    client::PimdirStore,
    collection::PimdirCollectionId,
    object::PimdirObject,
    placement::{PimdirHandle, PimdirLevel, PimdirPlacement, PimdirSortKey, PimdirStatus},
    summary,
};
use serde_json::{Value, json};

/// Held by every test of the file, so none forks the binary while another
/// holds or takes the owner lock.
static SERIAL: Mutex<()> = Mutex::new(());

/// The body [`filed`] stores for a message `id`.
fn body(id: &str) -> Vec<u8> {
    format!(
        "Message-ID: <{id}@example.org>\r\nSubject: {id}\r\nFrom: <{id}@example.org>\r\n\
         Date: Sat, 01 Aug 2026 10:00:00 +0000\r\n\r\n{id} words\r\n"
    )
    .into_bytes()
}

/// Files a message `id` into INBOX as source `local`, answering its
/// public id.
fn filed(dir: &Path, id: &str) -> i64 {
    let body = body(id);
    let mut store = PimdirStore::open(dir).unwrap().for_source("local");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    let derived = summary::derive("message/rfc822", &body).unwrap();
    let hash = store.hash(&body);
    store
        .write(vec![
            PimdirWriteOp::StoreObject {
                object: PimdirObject {
                    hash: hash.clone(),
                    size: body.len(),
                },
                body: Some(body),
            },
            PimdirWriteOp::UpsertPlacement(PimdirPlacement {
                sort_key: PimdirSortKey(derived.sort_key.as_str().into()),
                collection: PimdirCollectionId("INBOX".into()),
                handle: PimdirHandle(id.into()),
                link_id: Some(derived.link_id),
                level: PimdirLevel::Full,
                object: Some(hash),
                summary: derived.summary,
                flags: Default::default(),
                status: PimdirStatus::Clean,
                conflict_revision: None,
                conflict_object: None,
                base: None,
                origin: None,
            }),
        ])
        .unwrap();
    store
        .list_items("INBOX", None, 10)
        .unwrap()
        .into_iter()
        .find(|item| item.link_id.0.contains(id))
        .unwrap()
        .seq
}

/// Runs the binary over the store at `dir` with `--json`, answering its
/// exit status and its output parsed.
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

/// Without `--output`, `item export` prints the body as base64 `data`
/// with its `size` and the collection's media type, never raw bytes.
#[test]
fn an_exported_body_travels_as_data() {
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let seq = filed(dir.path(), "a");

    let (status, output) = pimdir(dir.path(), &["item", "export", &seq.to_string()]);

    assert_eq!(status, 0, "{output}");
    assert_eq!(
        output,
        json!({
            "data": "TWVzc2FnZS1JRDogPGFAZXhhbXBsZS5vcmc+DQpTdWJqZWN0OiBhDQpGcm9tOiA8YUBleGFtcGxlLm9yZz4NCkRhdGU6IFNhdCwgMDEgQXVnIDIwMjYgMTA6MDA6MDAgKzAwMDANCg0KYSB3b3Jkcw0K",
            "size": body("a").len(),
            "mime": "message/rfc822",
        })
    );
}

/// `item list` prints `nextPage` while items follow, takes it back
/// through `--page-token`, and prints `null` on the last page.
#[test]
fn a_listing_resumes_at_its_next_page() {
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let mut filed: Vec<i64> = ["a", "b", "c"]
        .into_iter()
        .map(|id| filed(dir.path(), id))
        .collect();
    filed.sort();
    let seqs = |output: &Value| -> Vec<i64> {
        output["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["seq"].as_i64().unwrap())
            .collect()
    };

    let (status, first) = pimdir(dir.path(), &["item", "list", "INBOX", "--limit", "2"]);
    assert_eq!(status, 0, "{first}");
    let token = first["nextPage"].as_str().expect("a page follows");

    let (status, last) = pimdir(
        dir.path(),
        &[
            "item",
            "list",
            "INBOX",
            "--limit",
            "2",
            "--page-token",
            token,
        ],
    );
    assert_eq!(status, 0, "{last}");
    assert_eq!(last["nextPage"], Value::Null);

    let mut listed = [seqs(&first), seqs(&last)].concat();
    listed.sort();
    assert_eq!(listed, filed, "every item once, across both pages");

    let (status, refused) = pimdir(
        dir.path(),
        &["item", "list", "INBOX", "--page-token", "nonsense"],
    );
    assert_eq!(
        (status, refused["code"].as_str()),
        (1, Some("page-token-invalid"))
    );
}

/// A binary's own types print every key, `null` when the value is absent.
#[test]
fn an_absent_value_prints_null() {
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    filed(dir.path(), "a");

    let (status, listed) = pimdir(dir.path(), &["collection", "list"]);
    assert_eq!(status, 0, "{listed}");
    let row = &listed["collections"][0];
    for key in [
        "description",
        "color",
        "role",
        "coveredSince",
        "coveredUntil",
        "coveredAt",
    ] {
        assert_eq!(row.get(key), Some(&Value::Null), "{key} in {row}");
    }
    assert_eq!(row["default"], false);

    let (status, unknown) = pimdir(dir.path(), &["queue", "status", "999"]);
    assert_eq!(status, 0, "{unknown}");
    assert_eq!(
        unknown,
        json!({
            "row": 999,
            "status": "unknown",
            "collection": null,
            "kind": null,
            "attempts": null,
            "error": null,
            "appliedAt": null,
            "seq": null,
        })
    );
}

/// `check` prints its result whatever it finds, and exits 2 while a
/// problem it does not repair is left.
#[test]
fn a_check_leaving_a_problem_exits_2() {
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    filed(dir.path(), "a");

    let (status, clean) = pimdir(dir.path(), &["check"]);
    assert_eq!(status, 0, "{clean}");

    let quarantine = dir.path().join("quarantine");
    fs::create_dir_all(&quarantine).unwrap();
    fs::write(quarantine.join("a.1-1-1"), b"failing").unwrap();

    let (status, found) = pimdir(dir.path(), &["check", "--fix"]);
    assert_eq!(status, 2, "{found}");
    assert_eq!(found["quarantined"].as_array().unwrap().len(), 1);
}
