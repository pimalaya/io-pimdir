//! The in-memory card matcher against the format's search vectors (pimdir
//! SEARCH §11): every case a set of cards can answer, one parsed and
//! reading no store, answers over the fixture store's cards the hits the
//! case expects among them, as an index holding those cards does.
//!
//! The vectors are the vendored copy under spec/vectors/, so the suite
//! always runs.

#![cfg(feature = "contact")]

use std::{collections::BTreeSet, fs, path::PathBuf};

use io_pimdir::search::{card::PimdirCard, query::PimdirQuery};
use serde_json::Value;

fn vectors(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("spec/vectors");
    serde_json::from_str(&fs::read_to_string(path.join(name)).unwrap()).unwrap()
}

/// Every contact case answers among the fixture's cards what the index
/// answers among them.
#[test]
fn every_contact_case_answers_the_index_hits_among_the_cards() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("spec/vectors");
    let store = vectors("search/store.json");
    let queries = vectors("search/queries.json");

    let contacts: BTreeSet<&str> = store["collections"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|collection| collection["kind"] == "text/vcard")
        .map(|collection| collection["id"].as_str().unwrap())
        .collect();
    let held: Vec<(String, i64, PimdirCard)> = store["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| contacts.contains(item["collection"].as_str().unwrap()))
        .map(|item| {
            let collection = item["collection"].as_str().unwrap();
            let account = store["collections"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == collection)
                .and_then(|c| c["account"].as_str())
                .unwrap_or_default()
                .to_string();
            let body = fs::read(root.join(item["fixture"].as_str().unwrap())).unwrap();
            (
                account,
                item["seq"].as_i64().unwrap(),
                PimdirCard::read(&body),
            )
        })
        .collect();
    assert!(held.len() >= 3, "the fixture store holds cards");
    let cards: Vec<PimdirCard> = held.iter().map(|(_, _, card)| card.clone()).collect();

    let mut answered = 0;
    for case in queries["cases"].as_array().unwrap() {
        let Some(hits) = case["hits"].as_array() else {
            continue;
        };
        let Ok(query) = PimdirQuery::parse(case["query"].as_str().unwrap()) else {
            continue;
        };
        if !query.store_terms().is_empty() {
            continue;
        }

        let want: BTreeSet<(String, i64)> = hits
            .iter()
            .map(|hit| {
                (
                    hit[0].as_str().unwrap().to_string(),
                    hit[1].as_i64().unwrap(),
                )
            })
            .filter(|hit| {
                held.iter()
                    .any(|(account, seq, _)| (account, seq) == (&hit.0, &hit.1))
            })
            .collect();
        let got: BTreeSet<(String, i64)> = query
            .match_cards(&cards)
            .unwrap()
            .into_iter()
            .zip(&held)
            .filter(|(hit, _)| *hit)
            .map(|(_, (account, seq, _))| (account.clone(), *seq))
            .collect();
        assert_eq!(got, want, "{}", case["query"]);
        answered += 1;
    }
    assert!(answered > 100, "{answered} cases answered");
}
