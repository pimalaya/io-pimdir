//! The crate's queue payload codec against the format's own vectors
//! (pimdir STORAGE.md §15.2, §15.3, Annex B.2): every payload of
//! vectors/payloads.json is judged valid, malformed or unknown as its case
//! states, a valid one reading, a malformed one refused and an unknown one
//! kept verbatim, read no further.
//!
//! The vectors are the vendored copy under spec/vectors/, so the suite
//! always runs; a case a later phase fixes is on tests/pending_vectors.txt.

mod pending;

use std::{fs, path::PathBuf};

use io_pimdir::{
    codec::{self, PimdirAction, PimdirActionError, PimdirPayloadOutcome},
    intent::{
        PimdirCollectionCreate, PimdirCollectionDelete, PimdirCollectionRename, PimdirInvitation,
        PimdirSubmit,
    },
};
use serde_json::Value;

#[test]
fn every_payload_vector_meets_its_outcome() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("spec/vectors/payloads.json");
    let vectors: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(vectors["v"], 2, "payloads.json at another version");

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

/// One case's payload meets the outcome the case states, by the queue
/// codec a producer and the drain read it with: a valid one reads, a
/// malformed one is refused, an unknown one is kept verbatim as
/// [`PimdirAction::Unknown`] whatever its other members hold. An intent's
/// is read by its type too, which a performer reads it with: it agrees
/// with the codec on a payload this crate knows, and refuses one at a
/// version it does not know rather than read it as `v` 1.
fn reads(case: &Value) {
    let action = case["action"].as_str().unwrap();
    let payload = case["payload"].to_string();
    let read = codec::action_from_payload(action, &payload).map_err(|err| err.to_string());
    let outcome = codec::payload_outcome(action, &payload);

    let typed = match action {
        "calendar-reply" | "calendar-cancel" => {
            Some(PimdirInvitation::from_payload(action, &payload).map(drop))
        }
        "collection-create" => Some(PimdirCollectionCreate::from_payload(&payload).map(drop)),
        "collection-rename" => Some(PimdirCollectionRename::from_payload(&payload).map(drop)),
        "collection-delete" => Some(PimdirCollectionDelete::from_payload(&payload).map(drop)),
        "submit" => Some(PimdirSubmit::from_payload(&payload).map(drop)),
        _ => None,
    };

    match case["outcome"].as_str().unwrap() {
        "valid" => {
            assert_eq!(
                outcome,
                PimdirPayloadOutcome::Valid,
                "{action} {payload}: {read:?}"
            );
            assert!(read.is_ok(), "{action} {payload}: refused: {read:?}");
            if let Some(typed) = typed {
                assert!(typed.is_ok(), "{action} {payload}: its type refuses it");
            }
        }
        "malformed" => {
            assert_eq!(
                outcome,
                PimdirPayloadOutcome::Malformed,
                "{action} {payload}"
            );
            assert!(read.is_err(), "{action} {payload}: read");
            if let Some(typed) = typed {
                assert_eq!(
                    typed.map_err(|err| err.to_string()),
                    read.map(drop),
                    "{action} {payload}: the codec and the type"
                );
            }
        }
        "unknown" => {
            assert_eq!(outcome, PimdirPayloadOutcome::Unknown, "{action} {payload}");
            let kept = PimdirAction::Unknown {
                kind: action.to_string(),
                payload: payload.clone(),
                object_hash: None,
            };
            assert_eq!(read, Ok(kept), "{action} {payload}: kept verbatim");
            if let Some(typed) = typed {
                assert!(
                    matches!(typed, Err(PimdirActionError::UnknownVersion(_))),
                    "{action} {payload}: its type reads another version"
                );
            }
        }
        outcome => panic!("{action} {payload}: an outcome of another name, {outcome}"),
    }
}
