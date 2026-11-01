//! The CLI's erasure and quarantine verbs (pimdir STORAGE §7, §11.5), run
//! as the binary over a temporary store with `--json`: `item erase` erases
//! a retained mail from both files and refuses a live one a source syncs,
//! `quarantine empty` unlinks what the owner moved there.
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
    change::{PimdirDropReason, PimdirWriteOp},
    client::{
        PimdirStore,
        search::{PimdirIndex, PimdirIndexRefreshOptions},
    },
    collection::PimdirCollectionId,
    object::PimdirObject,
    placement::{PimdirHandle, PimdirLevel, PimdirPlacement, PimdirSortKey, PimdirStatus},
    summary,
};
use serde_json::Value;

/// Held by every test of the file, so none forks the binary while another
/// holds or takes the owner lock.
static SERIAL: Mutex<()> = Mutex::new(());

/// Files a message `id` into INBOX as source `local`, indexes the store,
/// and answers its public id.
fn filed(dir: &Path, id: &str) -> i64 {
    let body = format!(
        "Message-ID: <{id}@example.org>\r\nSubject: {id}\r\nFrom: <{id}@example.org>\r\n\
         Date: Sat, 01 Aug 2026 10:00:00 +0000\r\n\r\n{id} words\r\n"
    )
    .into_bytes();
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
    let seq = store
        .list_items("INBOX", None, 10)
        .unwrap()
        .into_iter()
        .find(|item| item.link_id.0.contains(id))
        .unwrap()
        .seq;
    drop(store);

    PimdirIndex::open(dir)
        .unwrap()
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    seq
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

/// `item erase` refuses a live item a source syncs, and erases a retained
/// one from the store and its index, asking for `--yes` first.
#[test]
fn the_erase_verb_erases_a_retained_mail_from_both_files() {
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let seq = filed(dir.path(), "wombat");
    let seq = seq.to_string();

    let (status, _) = pimdir(dir.path(), &["item", "erase", &seq, "--yes"]);
    assert_eq!(status, 1, "a live item a source syncs is refused");

    PimdirStore::open(dir.path())
        .unwrap()
        .for_source("local")
        .write(vec![PimdirWriteOp::DropPlacement {
            collection: PimdirCollectionId("INBOX".into()),
            handle: PimdirHandle("wombat".into()),
            reason: PimdirDropReason::Deleted,
        }])
        .unwrap();

    let (status, _) = pimdir(dir.path(), &["item", "erase", &seq]);
    assert_eq!(status, 1, "no erase without --yes in a pipe");

    let (status, output) = pimdir(dir.path(), &["item", "erase", &seq, "--yes"]);
    assert_eq!(status, 0, "{output}");
    assert_eq!(output["collection"], "INBOX");
    assert_eq!(output["blobs"], 1);
    assert_eq!(output["index"], true);

    for name in ["pimdir.db", "index.db"] {
        let bytes = fs::read(dir.path().join(name)).unwrap();
        assert!(
            !bytes.windows(6).any(|window| window == b"wombat"),
            "wombat left in {name}"
        );
    }
}

/// `quarantine empty` unlinks every quarantined file, after `--yes`.
#[test]
fn the_quarantine_verb_empties_it() {
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    drop(PimdirStore::open(dir.path()).unwrap());
    let quarantine = dir.path().join("quarantine");
    fs::create_dir(&quarantine).unwrap();
    fs::write(quarantine.join("a.1-1-1"), b"failing").unwrap();

    let (status, _) = pimdir(dir.path(), &["quarantine", "empty"]);
    assert_eq!(status, 1, "no emptying without --yes in a pipe");
    assert!(quarantine.join("a.1-1-1").exists());

    let (status, output) = pimdir(dir.path(), &["quarantine", "empty", "--yes"]);
    assert_eq!(status, 0, "{output}");
    assert_eq!(output["files"], 1);
    assert_eq!(output["bytes"], 7);
    assert_eq!(fs::read_dir(&quarantine).unwrap().count(), 0);
}
