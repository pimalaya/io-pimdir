//! The CLI's writing search verb, `tag`, and `thread`, run as the binary
//! over a temporary store with `--json` (pimdir SEARCH.md §9, §10): what
//! it stages, what a drain applies, what waits while another process
//! owns the store.
//!
//! A file of its own, so no test forking the binary shares a process with
//! tests taking the owner lock: a child holds the parent's descriptors
//! until it executes, a lock among them. The tests here take [`SERIAL`]
//! for the same reason.

#![cfg(feature = "cli")]

use std::{
    collections::BTreeMap,
    path::Path,
    process::Command,
    sync::{Mutex, PoisonError},
};

use io_pimdir::{
    change::PimdirWriteOp,
    client::PimdirStore,
    collection::PimdirCollectionId,
    object::PimdirObject,
    placement::{
        PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement, PimdirSortKey, PimdirStatus,
    },
    summary,
};
use serde_json::Value;

/// Held by every test of the file, so none forks the binary while another
/// holds or takes the owner lock.
static SERIAL: Mutex<()> = Mutex::new(());

/// Files a message, `id` sent at `hour`, into a collection as source
/// `local`.
fn file_message(dir: &Path, collection: &str, id: &str, hour: u32, flags: &[&str]) {
    file_message_as(dir, "local", collection, id, hour, flags);
}

/// Files a message into a collection as `source`.
fn file_message_as(
    dir: &Path,
    source: &str,
    collection: &str,
    id: &str,
    hour: u32,
    flags: &[&str],
) {
    let body = format!(
        "Message-ID: <{id}>\r\nSubject: {id}\r\nFrom: Ann <ann@example.org>\r\n\
         Date: Sat, 01 Aug 2026 {hour:02}:00:00 +0000\r\n\r\nText\r\n"
    )
    .into_bytes();
    let mut store = PimdirStore::open(dir).unwrap().for_source(source);
    store
        .ensure_collection(collection, "message/rfc822")
        .unwrap();
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
                collection: PimdirCollectionId(collection.into()),
                handle: PimdirHandle(id.into()),
                link_id: Some(PimdirLinkId(id.into())),
                level: PimdirLevel::Full,
                object: Some(hash),
                summary: derived.summary,
                flags: flags.iter().copied().collect(),
                status: PimdirStatus::Clean,
                conflict_revision: None,
                conflict_object: None,
                base: None,
                origin: None,
            }),
        ])
        .unwrap();
}

/// The flags the store holds for the item `seq` in `collection`.
fn flags(dir: &Path, collection: &str, seq: i64) -> Vec<String> {
    PimdirStore::open(dir)
        .unwrap()
        .get_item(collection, seq)
        .unwrap()
        .unwrap()
        .flags
        .known()
        .unwrap()
        .iter()
        .cloned()
        .collect()
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

/// The statuses of a `tag` output's placements, by collection.
fn statuses(output: &Value) -> BTreeMap<String, String> {
    output["hits"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|hit| hit["placements"].as_array().unwrap())
        .map(|placement| {
            (
                placement["collection"].as_str().unwrap().to_string(),
                placement["status"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn both(status: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("Archive".into(), status.into()),
        ("INBOX".into(), status.into()),
    ])
}

#[test]
fn the_tag_verb_previews_queues_while_held_and_applies_when_free() {
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    file_message(dir, "INBOX", "alpha@x", 9, &["\\Seen"]);
    file_message(dir, "Archive", "alpha@x", 9, &[]);
    file_message(dir, "INBOX", "beta@x", 10, &[]);
    let seq = PimdirStore::open(dir)
        .unwrap()
        .seq_for_link("INBOX", "alpha@x")
        .unwrap()
        .unwrap();
    let queue = || pimdir(dir, &["queue", "list"]).1.to_string();
    let empty = queue();

    let (status, refused) = pimdir(dir, &["tag", "foo:bar", "--add", "x"]);
    assert_eq!(
        (status, refused["code"].as_str()),
        (1, Some("query-refused"))
    );
    assert!(
        refused["error"].as_str().unwrap().contains("unknown-field"),
        "the failure names the fault: {refused}"
    );
    assert_eq!(queue(), empty, "a refused query stages nothing");

    let (status, dry) = pimdir(dir, &["tag", "alpha", "--add", "flagged", "--dry-run"]);
    assert_eq!((status, statuses(&dry)), (0, both("planned")));
    assert_eq!(queue(), empty, "a dry run stages nothing");

    let owner = PimdirStore::open(dir).unwrap();
    let (status, held) = pimdir(dir, &["tag", "alpha", "--add", "flagged"]);
    assert_eq!((status, statuses(&held)), (0, both("queued")));
    drop(owner);
    assert_eq!(flags(dir, "INBOX", seq), ["\\Seen"], "queued, not applied");
    assert_ne!(queue(), empty);

    let (status, free) = pimdir(dir, &["tag", "alpha", "--remove", "read"]);
    assert_eq!(status, 0);
    assert_eq!(
        statuses(&free),
        BTreeMap::from([
            ("Archive".into(), "unchanged".into()),
            ("INBOX".into(), "applied".into()),
        ]),
        "planned on the queued flags, so the queued tag is kept"
    );
    assert_eq!(flags(dir, "INBOX", seq), ["\\Flagged"]);
    assert_eq!(flags(dir, "Archive", seq), ["\\Flagged"]);
    assert_eq!(queue(), empty, "the drain applied both");

    let (status, thread) = pimdir(dir, &["thread", &seq.to_string()]);
    assert_eq!(status, 0);
    assert_eq!(
        (thread["thread"].as_str(), thread["count"].as_u64()),
        (Some("alpha@x"), Some(1))
    );
}

/// What a `tag` output says of each placement: its status and the
/// sources syncing its collection, by collection.
fn sourced(output: &Value) -> BTreeMap<String, (String, Value)> {
    output["hits"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|hit| hit["placements"].as_array().unwrap())
        .map(|placement| {
            (
                placement["collection"].as_str().unwrap().to_string(),
                (
                    placement["status"].as_str().unwrap().to_string(),
                    placement["sources"].clone(),
                ),
            )
        })
        .collect()
}

#[test]
fn a_tag_stages_each_placement_as_the_source_syncing_its_collection() {
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    file_message_as(dir, "imap", "INBOX", "alpha@x", 9, &[]);
    file_message_as(dir, "jmap", "Notes", "alpha@x", 9, &[]);
    file_message_as(dir, "imap", "Shared", "alpha@x", 9, &[]);
    file_message_as(dir, "jmap", "Shared", "alpha@x", 9, &[]);
    let seq = PimdirStore::open(dir)
        .unwrap()
        .seq_for_link("INBOX", "alpha@x")
        .unwrap()
        .unwrap();
    let entry =
        |status: &str, sources: &[&str]| (status.to_string(), Value::from(sources.to_vec()));

    let (status, tagged) = pimdir(dir, &["tag", "alpha", "--add", "flagged"]);
    assert_eq!(
        status, 2,
        "a placement of an ambiguous source is not tagged"
    );
    assert_eq!(
        sourced(&tagged),
        BTreeMap::from([
            ("INBOX".into(), entry("applied", &["imap"])),
            ("Notes".into(), entry("applied", &["jmap"])),
            (
                "Shared".into(),
                entry("ambiguous-source", &["imap", "jmap"])
            ),
        ])
    );
    assert_eq!(flags(dir, "Notes", seq), ["\\Flagged"]);
    assert!(flags(dir, "Shared", seq).is_empty(), "nothing staged there");

    let (status, picked) = pimdir(
        dir,
        &["tag", "alpha", "--add", "flagged", "--source", "jmap"],
    );
    assert_eq!(status, 0);
    assert_eq!(
        sourced(&picked),
        BTreeMap::from([
            ("INBOX".into(), entry("unchanged", &["imap"])),
            ("Notes".into(), entry("unchanged", &["jmap"])),
            ("Shared".into(), entry("applied", &["jmap"])),
        ])
    );
    assert_eq!(flags(dir, "Shared", seq), ["\\Flagged"]);

    let (status, elsewhere) = pimdir(
        dir,
        &[
            "tag",
            "alpha",
            "--add",
            "work",
            "--source",
            "jmap",
            "--dry-run",
        ],
    );
    assert_eq!(status, 2, "a placement of an unsynced source would not be");
    assert_eq!(
        sourced(&elsewhere)["INBOX"],
        entry("unsynced", &["imap"]),
        "never staged as a source not syncing the collection"
    );
}
