//! The crate's queue payload codec against the format's own vectors
//! (pimdir STORAGE.md §15.3, Annex B.2): every valid payload of
//! vectors/payloads.json reads, every malformed one is refused.
//!
//! The vectors are the vendored copy under spec/vectors/, so the suite
//! always runs; a case a later phase fixes is on tests/pending_vectors.txt.

mod pending;

use std::{fs, path::PathBuf};

use io_pimdir::{
    codec,
    intent::{
        PimdirCollectionCreate, PimdirCollectionDelete, PimdirCollectionRename, PimdirInvitation,
        PimdirSubmit,
    },
};
use serde_json::Value;

#[test]
fn every_payload_vector_reads_or_is_refused() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("spec/vectors/payloads.json");
    let vectors: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(vectors["v"], 1, "payloads.json at another version");

    let cases = vectors["cases"].as_array().unwrap();
    assert!(!cases.is_empty(), "the vectors carry no case");

    // NOTE: a case is named by its action and its payload, which no other
    // case of the file repeats.
    let outcomes = cases
        .iter()
        .map(|case| {
            let name = format!("{} {}", case["action"].as_str().unwrap(), case["payload"]);
            (name, pending::check(|| reads(case)))
        })
        .collect();
    pending::settle("payloads", outcomes);
}

/// One case's payload reads exactly when the case says it is valid, by
/// the queue codec a producer and the drain read it with, and an intent's
/// by its type too, which a performer reads it with.
fn reads(case: &Value) {
    let action = case["action"].as_str().unwrap();
    let payload = case["payload"].to_string();
    let read = codec::action_from_payload(action, &payload)
        .map(drop)
        .map_err(|err| err.to_string());
    let typed = match action {
        "calendar-reply" | "calendar-cancel" => PimdirInvitation::from_payload(action, &payload)
            .map(drop)
            .map_err(|err| err.to_string()),
        "collection-create" => PimdirCollectionCreate::from_payload(&payload)
            .map(drop)
            .map_err(|err| err.to_string()),
        "collection-rename" => PimdirCollectionRename::from_payload(&payload)
            .map(drop)
            .map_err(|err| err.to_string()),
        "collection-delete" => PimdirCollectionDelete::from_payload(&payload)
            .map(drop)
            .map_err(|err| err.to_string()),
        "submit" => PimdirSubmit::from_payload(&payload)
            .map(drop)
            .map_err(|err| err.to_string()),
        _ => read.clone(),
    };
    assert_eq!(read, typed, "{action} {payload}: the codec and the type");

    match case["valid"].as_bool().unwrap() {
        true => assert!(read.is_ok(), "{action} {payload}: refused: {read:?}"),
        false => assert!(read.is_err(), "{action} {payload}: read"),
    }
}
