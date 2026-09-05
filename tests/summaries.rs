//! The crate's Annex A derivations against the format's own vectors
//! (pimdir STORAGE.md §16): every fixture's key, summary row, address rows
//! and sort key, compared as parsed structures.
//!
//! The spec is a sibling checkout, so the suite skips when it is absent.

use std::{fs, path::PathBuf};

use io_pimdir::{
    hash::PimdirHashAlgo,
    placement::PimdirHandle,
    summary::{self, PimdirSummary},
};
use serde_json::{Map, Value, json};

fn spec_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .join("pimdir");
    dir.join("vectors/summaries.json").is_file().then_some(dir)
}

/// A summary as the columns of its table, the way a vector states it.
fn row(summary: &PimdirSummary) -> (&'static str, Value) {
    let time = |time: &Option<summary::calendar::PimdirTime>| {
        (
            json!(time.as_ref().map(|t| &t.value)),
            json!(time.as_ref().and_then(|t| t.tzid.clone())),
            json!(time.as_ref().map(|t| t.value_kind())),
        )
    };
    let flag = |flag: Option<bool>| json!(flag.map(i64::from));

    match summary {
        PimdirSummary::Mail(mail) => (
            "mail_summary",
            json!({
                "message_id": mail.message_id,
                "in_reply_to": mail.in_reply_to,
                "subject": mail.subject,
                "sender": mail.sender,
                "sender_name": mail.sender_name,
                "date": mail.date,
                "size": mail.size,
                "attachment": flag(mail.attachment),
            }),
        ),
        PimdirSummary::Contact(contact) => (
            "contact_summary",
            json!({
                "uid": contact.uid,
                "fn": contact.full_name,
                "kind": contact.kind,
                "org": contact.org,
            }),
        ),
        PimdirSummary::Event(event) => {
            let (dtstart, dtstart_tzid, dtstart_value) = time(&event.dtstart);
            (
                "event_summary",
                json!({
                    "uid": event.uid,
                    "summary": event.summary,
                    "location": event.location,
                    "dtstart": dtstart,
                    "dtstart_tzid": dtstart_tzid,
                    "dtstart_value": dtstart_value,
                    "dtend": event.dtend,
                    "recurring": flag(event.recurring),
                    "until": event.until,
                }),
            )
        }
        PimdirSummary::Task(task) => {
            let (dtstart, dtstart_tzid, dtstart_value) = time(&task.dtstart);
            let (due, due_tzid, due_value) = time(&task.due);
            (
                "task_summary",
                json!({
                    "uid": task.uid,
                    "summary": task.summary,
                    "dtstart": dtstart,
                    "dtstart_tzid": dtstart_tzid,
                    "dtstart_value": dtstart_value,
                    "due": due,
                    "due_tzid": due_tzid,
                    "due_value": due_value,
                    "status": task.status,
                    "completed": task.completed,
                    "percent": task.percent,
                    "recurring": flag(task.recurring),
                    "until": task.until,
                }),
            )
        }
        PimdirSummary::Journal(journal) => {
            let (dtstart, dtstart_tzid, dtstart_value) = time(&journal.dtstart);
            (
                "journal_summary",
                json!({
                    "uid": journal.uid,
                    "summary": journal.summary,
                    "dtstart": dtstart,
                    "dtstart_tzid": dtstart_tzid,
                    "dtstart_value": dtstart_value,
                }),
            )
        }
    }
}

/// The address rows a summary yields, positions counted per role.
fn addresses(summary: &PimdirSummary) -> Vec<Value> {
    let mut positions: Map<String, Value> = Map::new();
    summary
        .addresses()
        .into_iter()
        .map(|(role, address)| {
            let position = positions.entry(role.as_str()).or_insert(json!(0));
            let at = position.as_i64().unwrap();
            *position = json!(at + 1);
            json!({
                "role": role.as_str(),
                "position": at,
                "address": address.address,
                "name": address.name,
            })
        })
        .collect()
}

#[test]
fn every_summary_vector_derives() {
    let Some(spec) = spec_dir() else {
        eprintln!("skipped: no pimdir spec checkout beside this one");
        return;
    };

    let vectors: Value =
        serde_json::from_str(&fs::read_to_string(spec.join("vectors/summaries.json")).unwrap())
            .unwrap();
    let cases = vectors["cases"].as_array().unwrap();
    assert!(!cases.is_empty(), "the vectors carry no case");

    for case in cases {
        let label = case["label"].as_str().unwrap_or("");
        let body = fs::read(spec.join("vectors").join(case["fixture"].as_str().unwrap())).unwrap();

        assert_eq!(
            PimdirHashAlgo::Blake3.hash(&body).0,
            case["body"]["blake3"].as_str().unwrap(),
            "{label}: the fixture read differently from what the vector names"
        );
        assert_eq!(
            PimdirHashAlgo::Sha256_128.hash(&body).0,
            case["body"]["sha256-128"].as_str().unwrap(),
            "{label}: the fixture read differently from what the vector names"
        );

        let derivation = summary::derive(case["kind"].as_str().unwrap(), &body)
            .unwrap_or_else(|| panic!("{label}: no conventions for the kind"));

        // NOTE: every case pins its key, the writer-derived hash: one
        // included (STORAGE §16); a hint with a handle pins the minted one.
        let expected = case["link_id"].as_str().unwrap();
        match (case.get("hint"), case.get("handle")) {
            (Some(hint), Some(handle)) => {
                assert_eq!(
                    derivation.link_id.as_str(),
                    hint.as_str().unwrap(),
                    "{label}: hint"
                );
                let minted = derivation
                    .link_id
                    .minted(&PimdirHandle::from(handle.as_str().unwrap()));
                assert_eq!(minted.as_str(), expected, "{label}: minted key");
            }
            _ => assert_eq!(derivation.link_id.as_str(), expected, "{label}: link id"),
        }

        let summary = derivation
            .summary
            .as_ref()
            .unwrap_or_else(|| panic!("{label}: no summary row"));
        let (table, actual) = row(summary);
        assert_eq!(table, case["table"].as_str().unwrap(), "{label}: table");
        assert_eq!(actual, case["summary"], "{label}: summary row");
        assert_eq!(
            addresses(summary),
            case["addresses"].as_array().cloned().unwrap_or_default(),
            "{label}: address rows"
        );
        assert_eq!(
            derivation.sort_key.as_str(),
            case["sort_key"].as_str().unwrap_or(""),
            "{label}: sort key"
        );
    }
}
