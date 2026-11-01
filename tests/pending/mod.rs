//! The vector cases expected to fail until a later phase of
//! cairn/changes/the-crate-follows-storage-draft-08 lands (its decision 3):
//! tests/pending_vectors.txt names each, with the phase fixing it.
//!
//! A runner checks every case of its suite, a failure staying the case's
//! own, then settles the outcomes against the list: a listed case failing
//! is accepted, while a listed case passing (take it off the list), an
//! unlisted case failing and a listed case the suite does not hold each
//! fail the suite, every one named at once. The list only shrinks.

use std::{collections::BTreeMap, fs, panic, path::PathBuf};

/// The suites the list may name, one per vector runner.
const SUITES: [&str; 6] = [
    "objects",
    "summaries",
    "payloads",
    "sync",
    "queries",
    "chunks",
];

/// What one case's checks found: what failed, or nothing.
pub type Outcome = Result<(), String>;

/// Runs one case's checks, a panic read as its failure, so the next case
/// still runs.
pub fn check(case: impl FnOnce()) -> Outcome {
    panic::catch_unwind(panic::AssertUnwindSafe(case)).map_err(|err| {
        err.downcast_ref::<String>()
            .cloned()
            .or_else(|| {
                err.downcast_ref::<&str>()
                    .map(|message| message.to_string())
            })
            .unwrap_or_default()
    })
}

/// Settles the outcomes of every case of `suite`, by case name, against
/// the list, failing the suite on any departure from it.
pub fn settle(suite: &str, outcomes: Vec<(String, Outcome)>) {
    assert!(SUITES.contains(&suite), "unknown suite {suite}");
    let listed = listed(suite);

    let mut departures = Vec::new();
    for (case, outcome) in &outcomes {
        match (outcome, listed.get(case)) {
            (Ok(()), Some(phase)) => departures.push(format!(
                "{suite}: {case} passes, take it off the list (phase {phase})"
            )),
            (Err(failure), None) => departures.push(format!("{suite}: {case} fails: {failure}")),
            _ => (),
        }
    }
    for case in listed.keys() {
        if !outcomes.iter().any(|(ran, _)| ran == case) {
            departures.push(format!("{suite}: {case} is listed and holds no case"));
        }
    }

    assert!(
        departures.is_empty(),
        "{} departure(s) from tests/pending_vectors.txt:\n{}",
        departures.len(),
        departures.join("\n")
    );
}

/// The cases of `suite` the list names, each with its phase.
fn listed(suite: &str) -> BTreeMap<String, u32> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/pending_vectors.txt");
    let mut listed = BTreeMap::new();
    for line in fs::read_to_string(path).unwrap().lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parsed = line.split_once(' ').and_then(|(phase, case)| {
            let (named, case) = case.split_once(": ")?;
            Some((phase.parse::<u32>().ok()?, named, case))
        });
        let Some((phase, named, case)) = parsed else {
            panic!("tests/pending_vectors.txt: malformed line {line:?}");
        };
        assert!(
            SUITES.contains(&named),
            "tests/pending_vectors.txt: unknown suite in {line:?}"
        );
        if named == suite {
            let duplicate = listed.insert(case.to_string(), phase);
            assert!(
                duplicate.is_none(),
                "tests/pending_vectors.txt: {line:?} twice"
            );
        }
    }
    listed
}
