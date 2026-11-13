//! The crate's queue payload codec against the format's own vectors
//! (pimdir STORAGE.md §15.2, §15.3, Annex B.2): every payload of
//! vectors/payloads.json is judged valid, malformed or unknown as its case
//! states, a valid one reading, a malformed one refused and an unknown one
//! kept verbatim, read no further.
//!
//! The vectors are the vendored copy under spec/vectors/, so the suite
//! always runs; a case a later phase fixes is on tests/pending_vectors.txt.

mod pending;

use std::{fs, ops::Range, path::PathBuf};

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
    let text = fs::read_to_string(path).unwrap();
    let vectors: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(vectors["v"], 2, "payloads.json at another version");

    let cases = vectors["cases"].as_array().unwrap();
    assert!(!cases.is_empty(), "the vectors carry no case");

    // NOTE: the codec is handed each payload as the file writes it. Read
    // into a value and printed again, `1e0` is `1.0`, another spelling
    // than the one its case states (STORAGE §15.3).
    let written = written_payloads(&text);
    assert_eq!(written.len(), cases.len());
    assert!(written.iter().any(|payload| payload.contains("\"v\": 1e0")));

    // NOTE: a case is named by its action and its payload, which no other
    // case of the file repeats.
    let outcomes = cases
        .iter()
        .zip(written)
        .map(|(case, written)| {
            let read: Value = serde_json::from_str(written).unwrap();
            assert_eq!(read, case["payload"], "the payload of another case");
            let name = format!("{} {}", case["action"].as_str().unwrap(), case["payload"]);
            (name, pending::check(|| reads(case, written)))
        })
        .collect();
    pending::settle("payloads", outcomes);
}

/// The `payload` of every case of vectors/payloads.json, in order, each
/// as the file writes it.
fn written_payloads(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let member = |object: usize, name: &str| {
        members(bytes, object)
            .into_iter()
            .find(|(key, _)| *key == name.as_bytes())
            .unwrap_or_else(|| panic!("a member {name}"))
            .1
    };

    let cases = member(text.find('{').unwrap(), "cases");
    let mut payloads = Vec::new();
    let mut at = blank_end(bytes, cases.start + 1);
    while bytes[at] != b']' {
        payloads.push(&text[member(at, "payload")]);
        at = blank_end(bytes, value_end(bytes, at));
    }
    payloads
}

/// The members of the JSON object opening at `at`: each key, between its
/// quotes, and where its value is written.
fn members(text: &[u8], at: usize) -> Vec<(&[u8], Range<usize>)> {
    let mut members = Vec::new();
    let mut at = blank_end(text, at + 1);
    while text[at] != b'}' {
        let key = at + 1..value_end(text, at) - 1;
        let value = blank_end(text, key.end + 1);
        let end = value_end(text, value);
        members.push((&text[key], value..end));
        at = blank_end(text, end);
    }
    members
}

/// The first octet at or after `at` that is neither whitespace nor the
/// comma or the colon between two values.
fn blank_end(text: &[u8], at: usize) -> usize {
    let blank = |byte: &u8| byte.is_ascii_whitespace() || matches!(byte, b',' | b':');
    at + text[at..].iter().take_while(|byte| blank(byte)).count()
}

/// The octet after the JSON value opening at `at`.
fn value_end(text: &[u8], at: usize) -> usize {
    let mut end = at;
    let mut depth = 0usize;
    loop {
        match text[end] {
            b'"' => {
                end += 1;
                while text[end] != b'"' {
                    end += if text[end] == b'\\' { 2 } else { 1 };
                }
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' if depth == 0 => return end,
            b'}' | b']' => depth -= 1,
            b',' if depth == 0 => return end,
            byte if depth == 0 && byte.is_ascii_whitespace() => return end,
            _ => {}
        }
        end += 1;
        if depth == 0 && matches!(text[end - 1], b'"' | b'}' | b']') {
            return end;
        }
    }
}

/// One case's payload meets the outcome the case states, by the queue
/// codec a producer and the drain read it with: a valid one reads, a
/// malformed one is refused, an unknown one is kept verbatim as
/// [`PimdirAction::Unknown`] whatever its other members hold. An intent's
/// is read by its type too, which a performer reads it with: it agrees
/// with the codec on a payload this crate knows, and refuses one at a
/// version it does not know rather than read it as `v` 1. `payload` is
/// the case's as the file writes it.
fn reads(case: &Value, payload: &str) {
    let action = case["action"].as_str().unwrap();
    let payload = payload.to_string();
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
