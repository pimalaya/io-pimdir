//! The in-memory mail matcher against the format's search vectors (pimdir
//! SEARCH §11): every case a set of messages can answer, one parsed and
//! naming no term a message cannot answer alone, answers over the fixture
//! store's messages the hits the case expects among them, as an index
//! holding those messages does.
//!
//! A hit's placements are one message carrying the union of their flags,
//! as a hit answers `tag:` (SEARCH §8.2). A bodiless or unreadable item,
//! which the index reads from a summary the vectors state rather than
//! derive, is left out of both sides.

#![cfg(feature = "mail")]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

use io_pimdir::search::{mail::PimdirMail, query::PimdirQuery};
use jiff::{Timestamp, tz::TimeZone};
use serde_json::Value;

fn vectors(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("spec/vectors");
    serde_json::from_str(&fs::read_to_string(path.join(name)).unwrap()).unwrap()
}

fn instant(value: &Value) -> i64 {
    value
        .as_str()
        .unwrap()
        .parse::<Timestamp>()
        .unwrap()
        .as_second()
}

/// Every mail case answers among the fixture's messages what the index
/// answers among them.
#[test]
fn every_mail_case_answers_the_index_hits_among_the_messages() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("spec/vectors");
    let store = vectors("search/store.json");
    let queries = vectors("search/queries.json");

    let collections: BTreeMap<&str, (&str, &str)> = store["collections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let id = c["id"].as_str().unwrap();
            let account = c["account"].as_str().unwrap_or_default();
            (id, (account, c["kind"].as_str().unwrap()))
        })
        .collect();

    let mut bodies: BTreeMap<(String, i64), (PathBuf, BTreeSet<String>)> = BTreeMap::new();
    for item in store["items"].as_array().unwrap() {
        let (account, kind) = collections[item["collection"].as_str().unwrap()];
        let Some(fixture) = item["fixture"].as_str() else {
            continue;
        };
        if kind != "message/rfc822" || item.get("unreadable").is_some() {
            continue;
        }
        let key = (account.to_string(), item["seq"].as_i64().unwrap());
        let entry = bodies
            .entry(key)
            .or_insert_with(|| (root.join(fixture), BTreeSet::new()));
        for flag in item["flags"].as_array().unwrap() {
            entry.1.insert(flag.as_str().unwrap().to_string());
        }
    }
    let held: Vec<((String, i64), PimdirMail)> = bodies
        .into_iter()
        .map(|(hit, (path, flags))| {
            let body = fs::read(path).unwrap();
            (hit, PimdirMail::read(&body, flags.into_iter().collect()))
        })
        .collect();
    assert!(held.len() >= 8, "the fixture store holds messages");
    let mails: Vec<PimdirMail> = held.iter().map(|(_, mail)| mail.clone()).collect();

    let mut answered = 0;
    for case in queries["cases"].as_array().unwrap() {
        let Some(hits) = case["hits"].as_array() else {
            continue;
        };
        let Ok(query) = PimdirQuery::parse(case["query"].as_str().unwrap()) else {
            continue;
        };
        if !query.mail_terms().is_empty() {
            continue;
        }
        let now = instant(case.get("now").unwrap_or(&queries["now"]));
        let zone = case["zone"]
            .as_str()
            .map_or(TimeZone::UTC, |zone| TimeZone::get(zone).unwrap());

        let want: BTreeSet<(String, i64)> = hits
            .iter()
            .map(|hit| {
                (
                    hit[0].as_str().unwrap().to_string(),
                    hit[1].as_i64().unwrap(),
                )
            })
            .filter(|hit| held.iter().any(|(held, _)| held == hit))
            .collect();
        let got: BTreeSet<(String, i64)> = query
            .match_mails(&mails, now, &zone)
            .unwrap()
            .into_iter()
            .zip(&held)
            .filter(|(hit, _)| *hit)
            .map(|(_, (hit, _))| hit.clone())
            .collect();
        assert_eq!(got, want, "{}", case["query"]);
        answered += 1;
    }
    assert!(answered > 100, "{answered} cases answered");
}
