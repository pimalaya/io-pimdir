//! The search index against the format's own vectors (pimdir SEARCH.md
//! §11): the fixture store of vectors/search/store.json built through the
//! owner, indexed, and asked every case of vectors/search/queries.json;
//! the chunk boundaries of vectors/search/chunks.json.
//!
//! Every case is answered: none is skipped. A case stating `threads` is
//! asked as a threaded listing (SEARCH §9).
//!
//! The spec is a sibling checkout, so the suite skips when it is absent.

#![cfg(feature = "search")]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use io_pimdir::{
    capability::{self, PimdirCapability, PimdirSupport},
    change::{PimdirDropReason, PimdirWriteOp},
    client::{
        PimdirError, PimdirStore,
        producer::PimdirProducer,
        search::{
            PimdirIndex, PimdirIndexRefreshOptions, PimdirSearchClock, PimdirSearchMatched,
            PimdirSearchOptions, PimdirSearchSort,
            tag::{PimdirSearchTagError, PimdirSearchTagPlan},
        },
    },
    collection::{PimdirCollectionId, PimdirScope},
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement, PimdirSortKey,
        PimdirStatus,
    },
    reference::{PimdirEndpoint, PimdirReferenceOrigin, PimdirReferenceRole},
    search::{
        chunk,
        coverage::PimdirSearchTemporal,
        query::{PimdirQuery, PimdirQueryPredicate},
    },
    summary::{
        self, PimdirAddress, PimdirSummary, file::PimdirFileSummary, mail::PimdirMailSummary,
    },
};
use serde_json::Value;

fn spec_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .join("pimdir");
    dir.join("vectors/search/queries.json")
        .is_file()
        .then_some(dir)
}

fn json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|values| {
            values
                .iter()
                .map(|value| value.as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// One placement, the owner's write of what a source listed.
fn placement(
    collection: &str,
    handle: &str,
    link_id: &str,
    object: Option<PimdirHash>,
    summary: PimdirSummary,
    sort_key: &str,
    flags: PimdirFlags,
) -> PimdirPlacement {
    PimdirPlacement {
        sort_key: PimdirSortKey(sort_key.into()),
        collection: PimdirCollectionId(collection.into()),
        handle: PimdirHandle(handle.into()),
        link_id: Some(PimdirLinkId(link_id.into())),
        level: match object {
            Some(_) => PimdirLevel::Full,
            None => PimdirLevel::Meta,
        },
        object,
        summary: Some(summary),
        flags,
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: None,
        origin: None,
    }
}

/// A bodiless mail summary as a vector states it.
fn mail_summary(row: &Value, addresses: &[Value]) -> PimdirSummary {
    let role = |role: &str| -> Vec<PimdirAddress> {
        addresses
            .iter()
            .filter(|address| address["role"] == role)
            .map(|address| PimdirAddress {
                address: address["address"].as_str().unwrap().into(),
                name: address["name"].as_str().map(Into::into),
            })
            .collect()
    };
    let text = |key: &str| row[key].as_str().map(String::from);

    PimdirSummary::Mail(PimdirMailSummary {
        message_id: text("message_id"),
        in_reply_to: strings(&row["in_reply_to"]),
        subject: text("subject").unwrap_or_default(),
        sender: text("sender"),
        sender_name: text("sender_name"),
        date: text("date"),
        size: row["size"].as_u64(),
        attachment: row["attachment"].as_i64().map(|mark| mark != 0),
        invitation: text("invitation"),
        from: role("from"),
        to: role("to"),
        cc: role("cc"),
        bcc: role("bcc"),
    })
}

/// Builds the fixture store at `dir` through the owner, checking every
/// item draws the seq the vector names.
fn build(spec: &Path, dir: &Path) {
    let store = json(&spec.join("vectors/search/store.json"));

    let mut kinds = BTreeMap::new();
    let mut stand_ins = BTreeSet::new();
    for collection in store["collections"].as_array().unwrap() {
        let id = collection["id"].as_str().unwrap();
        let kind = collection["kind"].as_str().unwrap();
        let owner = PimdirStore::open(dir)
            .unwrap()
            .for_account(collection["account"].as_str().unwrap());
        owner.ensure_collection(id, kind).unwrap();
        if let Some(role) = collection["role"].as_str() {
            owner.set_collection_role(id, Some(role)).unwrap();
            if role == "attachments" {
                stand_ins.insert(id.to_string());
            }
        }
        kinds.insert(id.to_string(), kind.to_string());
        drop(owner);

        // NOTE: a coverage row is what a source's rounds leave (STORAGE
        // §10): one closed, or one still open when none ever closed.
        for source in collection["sources"].as_array().into_iter().flatten() {
            let scope = PimdirScope {
                since: source["covered_since"].as_str().map(Into::into),
                until: source["covered_until"].as_str().map(Into::into),
            };
            let mut ops = vec![PimdirWriteOp::OpenRound {
                collection: PimdirCollectionId(id.into()),
                scope: scope.clone(),
                band: false,
            }];
            if !source["covered_at"].is_null() {
                ops.push(PimdirWriteOp::CloseRound {
                    collection: PimdirCollectionId(id.into()),
                    coverage: scope,
                    checkpoint: None,
                });
            }
            PimdirStore::open(dir)
                .unwrap()
                .for_account(collection["account"].as_str().unwrap())
                .for_source(source["source"].as_str().unwrap())
                .write(ops)
                .unwrap();
        }
    }

    let mut store_handle = PimdirStore::open(dir).unwrap().for_source("vectors");
    for item in store["items"].as_array().unwrap() {
        let collection = item["collection"].as_str().unwrap();
        let link_id = item["link_id"].as_str().unwrap();
        let seq = item["seq"].as_i64().unwrap();
        let flags: PimdirFlags = strings(&item["flags"]).into_iter().collect();
        let kind = &kinds[collection];

        if stand_ins.contains(collection) {
            // NOTE: the owner records a stand-in as its message's body
            // enters (STORAGE §14.3); the vector states what it left.
        } else if let Some(fixture) = item["fixture"].as_str() {
            let body = fs::read(spec.join("vectors").join(fixture)).unwrap();
            let derived = summary::derive(kind, &body).unwrap();
            assert_eq!(derived.link_id.as_str(), link_id, "{fixture}: link id");
            let hash = store_handle.hash(&body);
            store_handle
                .write(vec![
                    PimdirWriteOp::StoreObject {
                        object: PimdirObject {
                            hash: hash.clone(),
                            size: body.len(),
                        },
                        body: Some(body),
                    },
                    PimdirWriteOp::UpsertPlacement(placement(
                        collection,
                        &seq.to_string(),
                        link_id,
                        Some(hash),
                        derived.summary.unwrap(),
                        derived.sort_key.as_str(),
                        flags,
                    )),
                ])
                .unwrap();
        } else {
            let stated = &item["summary"];
            let row = &stated["row"];
            match stated["table"].as_str().unwrap() {
                "mail_summary" => {
                    let addresses = stated["addresses"].as_array().unwrap();
                    store_handle
                        .write(vec![PimdirWriteOp::UpsertPlacement(placement(
                            collection,
                            &seq.to_string(),
                            link_id,
                            None,
                            mail_summary(row, addresses),
                            stated["sort_key"].as_str().unwrap(),
                            flags,
                        ))])
                        .unwrap();
                }
                "file_summary" => {
                    let file = PimdirFileSummary {
                        name: row["name"].as_str().unwrap().into(),
                        media_type: row["media_type"].as_str().map(Into::into),
                        size: row["size"].as_u64(),
                        part: row["part"].as_str().map(Into::into),
                    };
                    store_handle
                        .put_file(collection, &PimdirLinkId(link_id.into()), &file)
                        .unwrap();
                }
                table => panic!("no fixture builder for {table}"),
            }
        }

        let placed = store_handle.seq_for_link(collection, link_id).unwrap();
        assert_eq!(placed, Some(seq), "{link_id} draws the vector's seq");
    }

    for reference in store["references"].as_array().unwrap() {
        let end = |kind: &str, link: &str| PimdirEndpoint {
            kind: reference[kind].as_str().unwrap().into(),
            link_id: PimdirLinkId(reference[link].as_str().unwrap().into()),
        };
        store_handle
            .add_reference(
                &end("from_kind", "from_link_id"),
                &end("to_kind", "to_link_id"),
                &PimdirReferenceRole::parse(reference["role"].as_str().unwrap()),
                PimdirReferenceOrigin::parse(reference["origin"].as_str().unwrap()),
            )
            .unwrap()
            .expect("the reference links");
    }
}

/// A hit list as `(account, seq)` pairs.
fn pairs(hits: &Value) -> Vec<(String, i64)> {
    hits.as_array()
        .unwrap()
        .iter()
        .map(|hit| {
            (
                hit[0].as_str().unwrap().to_string(),
                hit[1].as_i64().unwrap(),
            )
        })
        .collect()
}

#[test]
fn every_search_vector_answers() {
    let Some(spec) = spec_dir() else {
        eprintln!("skipped: no pimdir spec checkout beside this one");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    build(&spec, dir.path());
    let vectors = json(&spec.join("vectors/search/queries.json"));

    // NOTE: the horizon is drawn around the vectors' now (SEARCH §11).
    let now = unix(vectors["now"].as_str().unwrap());
    let clock = PimdirSearchClock { now: Some(now) };
    let mut index = PimdirIndex::open(dir.path()).unwrap();
    let report = index
        .refresh(&PimdirIndexRefreshOptions {
            now: Some(now),
            ..Default::default()
        })
        .unwrap();
    assert!(report.is_current(), "one pass folds the whole feed");
    let coverage: BTreeMap<String, u64> = vectors["coverage"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(kind, count)| (kind.clone(), count.as_u64().unwrap()))
        .collect();
    let store = json(&spec.join("vectors/search/store.json"));
    assert_eq!(
        bodiless(&store, &PimdirQuery::parse("changed:0..").unwrap()),
        coverage,
        "the store-wide coverage the vectors state"
    );
    let cases = vectors["cases"].as_array().unwrap();
    assert!(!cases.is_empty(), "the vectors carry no case");

    for case in cases {
        let query = case["query"].as_str().unwrap();

        if let Some(error) = case.get("error") {
            let refused = PimdirQuery::parse(query).expect_err(query);
            assert_eq!(
                (refused.code.as_str(), refused.start, refused.end),
                (
                    error["code"].as_str().unwrap(),
                    error["start"].as_u64().unwrap() as usize,
                    error["end"].as_u64().unwrap() as usize,
                ),
                "{query:?}: refusal"
            );
            if refused.code.as_str() == "unknown-field" {
                assert!(!refused.fields.is_empty(), "{query:?}: the known fields");
            }
            continue;
        }

        let parsed = PimdirQuery::parse(query).unwrap_or_else(|err| panic!("{query:?}: {err}"));

        let options = PimdirSearchOptions {
            sort: match case["sort"].as_str() {
                Some("name") => PimdirSearchSort::Name,
                Some("relevance") => PimdirSearchSort::Relevance,
                _ => PimdirSearchSort::Date,
            },
            limit: 100,
            offset: 0,
            clock,
        };
        if let Some(want) = case.get("threads") {
            let answer = index
                .threads(&parsed, &options)
                .unwrap_or_else(|err| panic!("{query:?}: {err}"));
            let got: BTreeSet<_> = answer
                .threads
                .iter()
                .map(|thread| {
                    let hits: BTreeSet<(String, i64)> = thread
                        .hits
                        .iter()
                        .map(|hit| (hit.account.clone().unwrap_or_default(), hit.seq))
                        .collect();
                    (
                        thread.thread.clone(),
                        thread.count,
                        thread.first.clone(),
                        thread.last.clone(),
                        hits,
                    )
                })
                .collect();
            let want: BTreeSet<_> = want
                .as_array()
                .unwrap()
                .iter()
                .map(|thread| {
                    (
                        thread["thread"].as_str().map(String::from),
                        thread["count"].as_u64().unwrap(),
                        thread["first"].as_str().map(String::from),
                        thread["last"].as_str().map(String::from),
                        pairs(&thread["hits"]).into_iter().collect::<BTreeSet<_>>(),
                    )
                })
                .collect();
            assert_eq!(got, want, "{query:?}: threads");
            continue;
        }
        let answer = index
            .search(&parsed, &options)
            .unwrap_or_else(|err| panic!("{query:?}: {err}"));
        let got: Vec<(String, i64)> = answer
            .hits
            .iter()
            .map(|hit| (hit.account.clone().unwrap_or_default(), hit.seq))
            .collect();
        let want = pairs(&case["hits"]);

        if case["ordered"].as_bool() == Some(true) {
            assert_eq!(got, want, "{query:?}: ordered hits");
        } else {
            assert_eq!(
                got.iter().collect::<BTreeSet<_>>(),
                want.iter().collect::<BTreeSet<_>>(),
                "{query:?}: hits"
            );
        }

        let count = index.count(&parsed, &clock).unwrap();
        assert_eq!(count.count as usize, want.len(), "{query:?}: count");

        let expected = bodiless(&store, &parsed);
        assert_eq!(
            answer.coverage.local.bodiless, expected,
            "{query:?}: coverage"
        );
        assert_eq!(answer.coverage.local.unindexed, 0, "{query:?}: unindexed");
        if let Some(temporal) = case.get("temporal") {
            let expected = match temporal.as_str() {
                Some("full") => PimdirSearchTemporal::Full,
                _ => PimdirSearchTemporal::Bounded {
                    since: temporal["since"].as_str().map(Into::into),
                    until: temporal["until"].as_str().map(Into::into),
                    filling: temporal["filling"].as_bool().unwrap(),
                },
            };
            assert_eq!(answer.coverage.temporal, expected, "{query:?}: temporal");
        }

        assert_eq!(
            answer.coverage.horizon_left,
            case["horizon_left"].as_bool().unwrap_or(false),
            "{query:?}: horizon_left"
        );
    }
}

#[test]
fn a_hit_says_what_it_matched_and_carries_a_snippet() {
    let Some(spec) = spec_dir() else {
        eprintln!("skipped: no pimdir spec checkout beside this one");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    build(&spec, dir.path());
    let mut index = PimdirIndex::open(dir.path()).unwrap();
    index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();

    let search = |query: &str| {
        index
            .search(
                &PimdirQuery::parse(query).unwrap(),
                &PimdirSearchOptions::default(),
            )
            .unwrap()
    };

    let budget = search("budget");
    assert_eq!(budget.hits[0].matched, Some(PimdirSearchMatched::Summary));
    assert_eq!(budget.hits[0].snippet, None, "a summary has no chunk");

    let reply = search("final");
    assert_eq!(reply.hits[0].matched, Some(PimdirSearchMatched::Body));
    assert_eq!(
        reply.hits[0].snippet.as_deref(),
        Some("Roadmap roadmap roadmap. The roadmap is not final.")
    );

    let structured = search("is:flagged");
    assert_eq!(structured.hits[0].matched, None, "no text term");
    assert!(
        structured.hits[0].snippet.is_some(),
        "the first chunk stands in"
    );

    let item = index.item(Some("home"), 13).unwrap().unwrap();
    assert_eq!(item.source, PimdirSearchMatched::Body);
    assert!(item.extract.attachment.contains("invoice.pdf"));
    assert_eq!(item.extract.body.trim(), "Attached.");
    assert!(index.item(Some("work"), 13).unwrap().is_none());

    let card = index.item(Some("home"), 5).unwrap().unwrap().extract;
    assert_eq!(card.title, "Doe, Jane\nJane\nDoe");
    assert_eq!(card.org, "Example, Inc.\nResearch");
    assert_eq!(card.email, "Jane.Doe@Example.COM");
    assert_eq!(
        card.note,
        "+33 6 12 34 56 78\n33612345678\nFirst linecontinued."
    );

    let event = index.item(Some("home"), 6).unwrap().unwrap().extract;
    assert_eq!(
        (event.title.as_str(), event.place.as_str()),
        ("Weekly sync", "Room 2")
    );
}

#[test]
fn every_chunk_vector_derives() {
    let Some(spec) = spec_dir() else {
        eprintln!("skipped: no pimdir spec checkout beside this one");
        return;
    };
    let vectors = json(&spec.join("vectors/search/chunks.json"));

    for case in vectors["cases"].as_array().unwrap() {
        let label = case["label"].as_str().unwrap();
        let mut got = Vec::new();
        for field in ["body", "note"] {
            let Some(parts) = case["fields"].get(field) else {
                continue;
            };
            let text: String = parts
                .as_array()
                .unwrap()
                .iter()
                .map(|part| match part.as_str() {
                    Some(text) => text.to_string(),
                    None => part["repeat"]
                        .as_str()
                        .unwrap()
                        .repeat(part["times"].as_u64().unwrap() as usize),
                })
                .collect();
            for range in chunk::chunks(&text) {
                got.push(serde_json::json!({
                    "ord": got.len(),
                    "field": field,
                    "start": range.start,
                    "end": range.end,
                }));
            }
        }
        assert_eq!(Value::from(got), case["chunks"], "{label}");
    }
}

/// A message with one subject and one body line.
fn message(id: &str, subject: &str, text: &str) -> Vec<u8> {
    format!(
        "Message-ID: <{id}>\r\nSubject: {subject}\r\nFrom: Ann <ann@example.org>\r\n\
         Date: Sat, 01 Aug 2026 12:00:00 +0000\r\n\r\n{text}\r\n"
    )
    .into_bytes()
}

/// Files a message into a collection as source `local`.
fn file_message(dir: &Path, collection: &str, body: Vec<u8>, flags: &[&str]) {
    let mut store = PimdirStore::open(dir).unwrap().for_source("local");
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
            PimdirWriteOp::UpsertPlacement(placement(
                collection,
                derived.link_id.as_str(),
                derived.link_id.as_str(),
                Some(hash),
                derived.summary.unwrap(),
                derived.sort_key.as_str(),
                flags.iter().copied().collect(),
            )),
        ])
        .unwrap();
}

fn hits(index: &PimdirIndex, query: &str) -> Vec<i64> {
    index
        .search(
            &PimdirQuery::parse(query).unwrap(),
            &PimdirSearchOptions::default(),
        )
        .unwrap()
        .hits
        .iter()
        .map(|hit| hit.seq)
        .collect()
}

#[test]
fn a_refresh_follows_the_feed_within_its_deadline() {
    let dir = tempfile::tempdir().unwrap();
    file_message(
        dir.path(),
        "INBOX",
        message("a@x", "Alpha", "first body"),
        &[],
    );
    file_message(
        dir.path(),
        "Archive",
        message("b@x", "Beta", "second body"),
        &[],
    );

    let mut index = PimdirIndex::open(dir.path()).unwrap();
    let past = PimdirIndexRefreshOptions {
        batch: 1,
        deadline: Some(std::time::Instant::now()),
        now: None,
    };
    let report = index.refresh(&past).unwrap();
    assert_eq!(
        (report.folded, report.cursor),
        (0, 0),
        "nothing past the deadline"
    );
    let stale = index
        .count(
            &PimdirQuery::parse("body").unwrap(),
            &PimdirSearchClock::default(),
        )
        .unwrap();
    assert_eq!(stale.count, 0);
    assert_eq!(
        stale.coverage.local.unindexed, 2,
        "the answer says what it missed"
    );
    assert!(!stale.coverage.is_complete());

    let report = index
        .refresh(&PimdirIndexRefreshOptions {
            batch: 1,
            ..Default::default()
        })
        .unwrap();
    assert!(report.is_current());
    assert_eq!((report.folded, report.indexed), (2, 2));
    assert_eq!(hits(&index, "body").len(), 2);
    assert!(
        index
            .count(
                &PimdirQuery::parse("body").unwrap(),
                &PimdirSearchClock::default()
            )
            .unwrap()
            .coverage
            .is_complete()
    );

    file_message(
        dir.path(),
        "INBOX",
        message("a@x", "Alpha", "first body"),
        &["\\Flagged"],
    );
    let report = index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    assert_eq!(
        (report.folded, report.indexed),
        (1, 0),
        "a body is indexed once"
    );
    assert_eq!(hits(&index, "is:flagged").len(), 1);

    PimdirStore::open(dir.path())
        .unwrap()
        .delete_collection("Archive")
        .unwrap();
    let report = index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    assert!(
        report.reconciled,
        "a purge moves the count the index follows"
    );
    assert_eq!(report.removed, 1);
    assert_eq!(hits(&index, "beta"), Vec::<i64>::new());
}

#[test]
fn an_index_built_otherwise_is_rebuilt_and_a_held_lock_skips_the_refresh() {
    let dir = tempfile::tempdir().unwrap();
    file_message(
        dir.path(),
        "INBOX",
        message("a@x", "Alpha", "first body"),
        &[],
    );

    let mut index = PimdirIndex::open(dir.path()).unwrap();
    index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    drop(index);

    let conn = rusqlite::Connection::open(dir.path().join("index.db")).unwrap();
    conn.execute("UPDATE index_meta SET tokenizer = 'porter'", [])
        .unwrap();
    drop(conn);

    let mut index = PimdirIndex::open(dir.path()).unwrap();
    assert_eq!(index.cursor().unwrap(), 0, "rebuilt, not migrated");

    let held = fs::File::create(dir.path().join("index.lock")).unwrap();
    fs4::FileExt::try_lock(&held).unwrap();
    let report = index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    assert!(report.busy && !report.is_current());
    drop(held);

    assert!(
        index
            .refresh(&PimdirIndexRefreshOptions::default())
            .unwrap()
            .is_current()
    );
    assert_eq!(hits(&index, "alpha").len(), 1);
}

/// The bodiless live items per kind a query's coverage counts, from the
/// vector store: those no file collection holds (SEARCH §8.2), in the
/// collections its top-level `account:`, `collection:` and `kind:` terms
/// narrow to.
fn bodiless(store: &Value, query: &PimdirQuery) -> BTreeMap<String, u64> {
    let conjuncts = match query {
        PimdirQuery::And(operands) => operands.iter().collect(),
        query => vec![query],
    };
    let collections = store["collections"].as_array().unwrap();
    let in_scope = |collection: &Value| {
        conjuncts.iter().all(|conjunct| {
            let PimdirQuery::Term(term) = conjunct else {
                return true;
            };
            match &term.predicate {
                PimdirQueryPredicate::Account(account) => collection["account"] == *account,
                PimdirQueryPredicate::Collection(id) => collection["id"] == *id,
                PimdirQueryPredicate::Kind(kind) => {
                    let media = match kind.as_str() {
                        "mail" => "message/rfc822",
                        "contact" => "text/vcard",
                        "file" => "application/octet-stream",
                        _ => "text/calendar",
                    };
                    collection["kind"] == media
                }
                _ => true,
            }
        })
    };

    let mut counts = BTreeMap::new();
    for item in store["items"].as_array().unwrap() {
        let collection = collections
            .iter()
            .find(|c| c["id"] == item["collection"])
            .unwrap();
        let kind = collection["kind"].as_str().unwrap();
        if item.get("fixture").is_none()
            && kind != "application/octet-stream"
            && in_scope(collection)
        {
            *counts.entry(kind.to_string()).or_default() += 1;
        }
    }
    counts
}

/// An RFC 3339 instant in UTC, `YYYY-MM-DDTHH:MM:SSZ`, as Unix seconds.
fn unix(instant: &str) -> i64 {
    let part = |range: core::ops::Range<usize>| instant[range].parse::<i64>().unwrap();
    let (year, month, day) = (part(0..4), part(5..7), part(8..10));

    // NOTE: days from the epoch to a civil date, proleptic Gregorian.
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let days = era * 146_097 + of_era * 365 + of_era / 4 - of_era / 100 + of_year - 719_468;

    days * 86_400 + part(11..13) * 3_600 + part(14..16) * 60 + part(17..19)
}

/// Every text field alone answers in every order: a term matching bodies
/// alone once ranked outside its full-text query, which FTS5 refuses.
#[test]
fn every_field_alone_answers_in_every_order() {
    let Some(spec) = spec_dir() else {
        eprintln!("skipped: no pimdir spec checkout beside this one");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    build(&spec, dir.path());
    let mut index = PimdirIndex::open(dir.path()).unwrap();
    index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();

    let queries = [
        "location:room",
        "location:\"Room 2\"",
        "attachment:invoice",
        "mimetype:pdf",
        "body:roadmap",
        "body:roa*",
        "title:sync",
        "subject:sync",
        "summary:sync",
        "with:alice",
        "from:alice",
        "to:bob",
        "cc:carol",
        "bcc:bob",
        "email:jane",
        "organizer:alice",
        "attendee:bob",
        "person:jane",
        "not body:roadmap",
    ];
    let sorts = [
        PimdirSearchSort::Date,
        PimdirSearchSort::Name,
        PimdirSearchSort::Relevance,
    ];
    for query in queries {
        let parsed = PimdirQuery::parse(query).unwrap();
        for sort in sorts {
            let options = PimdirSearchOptions {
                sort,
                ..Default::default()
            };
            if let Err(err) = index.search(&parsed, &options) {
                panic!("{query:?} sorted {sort:?}: {err}");
            }
        }
    }

    let room = index
        .search(
            &PimdirQuery::parse("location:room").unwrap(),
            &PimdirSearchOptions::default(),
        )
        .unwrap();
    assert_eq!(room.hits.len(), 1, "the event in Room 2");
}

/// A message dated 1 August 2026 at `hour`, `headers` before its own.
fn mail(id: &str, hour: u32, headers: &str) -> Vec<u8> {
    format!(
        "Message-ID: <{id}>\r\n{headers}Subject: {id}\r\nFrom: Ann <ann@example.org>\r\n\
         Date: Sat, 01 Aug 2026 {hour:02}:00:00 +0000\r\n\r\nText\r\n"
    )
    .into_bytes()
}

/// Deletes the message `id` filed by [`file_message`] in `INBOX`.
fn delete_message(dir: &Path, id: &str) {
    PimdirStore::open(dir)
        .unwrap()
        .for_source("local")
        .write(vec![PimdirWriteOp::DropPlacement {
            collection: PimdirCollectionId("INBOX".into()),
            handle: PimdirHandle(id.into()),
            reason: PimdirDropReason::Deleted,
        }])
        .unwrap();
}

/// Refreshes the index, then lists every message as threads: each its id,
/// count and the link ids of its hits.
fn threads(index: &mut PimdirIndex) -> BTreeSet<(String, u64, BTreeSet<String>)> {
    index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    index
        .threads(
            &PimdirQuery::parse("kind:mail").unwrap(),
            &PimdirSearchOptions::default(),
        )
        .unwrap()
        .threads
        .into_iter()
        .map(|thread| {
            let hits = thread
                .hits
                .iter()
                .map(|hit| hit.placements[0].link_id.0.clone())
                .collect();
            (thread.thread.unwrap(), thread.count, hits)
        })
        .collect()
}

/// The `thread` and `message` rows index.db holds.
fn thread_rows(dir: &Path) -> (i64, i64) {
    let conn = rusqlite::Connection::open(dir.join("index.db")).unwrap();
    let count = |table: &str| {
        conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    };
    (count("thread"), count("message"))
}

fn thread(id: &str, count: u64, hits: &[&str]) -> (String, u64, BTreeSet<String>) {
    (
        id.into(),
        count,
        hits.iter().map(|hit| hit.to_string()).collect(),
    )
}

#[test]
fn two_replies_to_a_parent_never_held_are_one_thread() {
    let dir = tempfile::tempdir().unwrap();
    for (id, hour) in [("c1@x", 10), ("c2@x", 11)] {
        let body = mail(
            id,
            hour,
            "In-Reply-To: <p@x>\r\nReferences: <r@x> <p@x>\r\n",
        );
        file_message(dir.path(), "INBOX", body, &[]);
    }

    let mut index = PimdirIndex::open(dir.path()).unwrap();
    assert_eq!(
        threads(&mut index),
        BTreeSet::from([thread("c1@x", 2, &["c1@x", "c2@x"])])
    );
    assert_eq!(
        thread_rows(dir.path()),
        (1, 3),
        "the parent nobody holds has a row; References is read only without In-Reply-To"
    );
    assert_eq!(hits(&index, "thread:c2@x").len(), 2, "any member names it");
}

#[test]
fn a_parent_arriving_later_joins_its_replies_and_names_the_thread() {
    let dir = tempfile::tempdir().unwrap();
    let reply = mail("c@x", 10, "References: <q@x> <p@x>\r\n");
    file_message(dir.path(), "INBOX", reply, &[]);
    let mut index = PimdirIndex::open(dir.path()).unwrap();
    assert_eq!(
        threads(&mut index),
        BTreeSet::from([thread("c@x", 1, &["c@x"])])
    );

    file_message(dir.path(), "INBOX", mail("p@x", 9, ""), &[]);
    assert_eq!(
        threads(&mut index),
        BTreeSet::from([thread("p@x", 2, &["p@x", "c@x"])]),
        "the last References id is the parent, the earliest member the name"
    );
    assert_eq!(thread_rows(dir.path()), (1, 2), "the old thread row went");
}

#[test]
fn a_message_joining_two_threads_renames_them_and_drops_the_old_row() {
    let dir = tempfile::tempdir().unwrap();
    file_message(dir.path(), "INBOX", mail("x@x", 10, ""), &[]);
    let reply = mail("y@x", 9, "In-Reply-To: <q@x>\r\n");
    file_message(dir.path(), "INBOX", reply, &[]);
    let mut index = PimdirIndex::open(dir.path()).unwrap();
    assert_eq!(
        threads(&mut index),
        BTreeSet::from([thread("x@x", 1, &["x@x"]), thread("y@x", 1, &["y@x"]),])
    );

    let bridge = mail("q@x", 11, "In-Reply-To: <x@x>\r\n");
    file_message(dir.path(), "INBOX", bridge, &[]);
    assert_eq!(
        threads(&mut index),
        BTreeSet::from([thread("y@x", 3, &["x@x", "y@x", "q@x"])])
    );
    assert_eq!(thread_rows(dir.path()), (1, 3));
    assert_eq!(
        hits(&index, "thread:x@x").len(),
        3,
        "the old id still reaches the thread"
    );
}

#[test]
fn a_cycle_is_broken_into_one_thread() {
    let dir = tempfile::tempdir().unwrap();
    let a = mail("a@x", 9, "In-Reply-To: <b@x>\r\n");
    let b = mail("b@x", 10, "In-Reply-To: <a@x>\r\n");
    let own = mail("o@x", 11, "In-Reply-To: <o@x>\r\n");
    for body in [a, b, own] {
        file_message(dir.path(), "INBOX", body, &[]);
    }

    let mut index = PimdirIndex::open(dir.path()).unwrap();
    assert_eq!(
        threads(&mut index),
        BTreeSet::from([
            thread("a@x", 2, &["a@x", "b@x"]),
            thread("o@x", 1, &["o@x"]),
        ])
    );
    assert_eq!(thread_rows(dir.path()), (2, 3));

    let conn = rusqlite::Connection::open(dir.path().join("index.db")).unwrap();
    let edges: i64 = conn
        .query_row(
            "SELECT count(*) FROM message WHERE parent IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(edges, 1, "one edge of the two, and no self edge, is kept");
}

#[test]
fn a_deleted_member_leaves_its_thread_consistent() {
    let dir = tempfile::tempdir().unwrap();
    file_message(dir.path(), "INBOX", mail("a@x", 8, ""), &[]);
    let b = mail("b@x", 9, "In-Reply-To: <a@x>\r\n");
    file_message(dir.path(), "INBOX", b, &[]);
    let c = mail("c@x", 10, "In-Reply-To: <b@x>\r\n");
    file_message(dir.path(), "INBOX", c, &[]);
    let mut index = PimdirIndex::open(dir.path()).unwrap();
    assert_eq!(
        threads(&mut index),
        BTreeSet::from([thread("a@x", 3, &["a@x", "b@x", "c@x"])])
    );

    delete_message(dir.path(), "a@x");
    assert_eq!(
        threads(&mut index),
        BTreeSet::from([thread("b@x", 2, &["b@x", "c@x"])]),
        "the earliest present member names it"
    );
    assert_eq!(thread_rows(dir.path()), (1, 3), "b still names a");

    delete_message(dir.path(), "b@x");
    assert_eq!(
        threads(&mut index),
        BTreeSet::from([thread("c@x", 1, &["c@x"])])
    );
    assert_eq!(
        thread_rows(dir.path()),
        (1, 2),
        "b stays as c's parent, a goes, named by nobody"
    );
}

/// The flags the store holds for the item `seq` in `collection`.
fn store_flags(dir: &Path, collection: &str, seq: i64) -> Vec<String> {
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

/// One message filed in `INBOX` and `Archive`, another in `INBOX`, the
/// index refreshed; the first one's seq.
fn tag_store(dir: &Path) -> (PimdirIndex, i64) {
    let alpha = || mail("alpha@x", 9, "");
    file_message(dir, "INBOX", alpha(), &["\\Seen"]);
    file_message(dir, "Archive", alpha(), &[]);
    file_message(dir, "INBOX", mail("beta@x", 10, ""), &[]);

    let mut index = PimdirIndex::open(dir).unwrap();
    index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    let seq = hits(&index, "id:alpha@x")[0];
    (index, seq)
}

#[test]
fn a_tag_stages_one_action_per_placement_and_a_drain_applies_it() {
    let dir = tempfile::tempdir().unwrap();
    let (index, seq) = tag_store(dir.path());
    let query = PimdirQuery::parse("subject:alpha").unwrap();
    let clock = PimdirSearchClock::default();

    let plan = index
        .tag(
            &query,
            &["flagged".into(), "work".into()],
            &["read".into()],
            None,
            &clock,
        )
        .unwrap();
    assert_eq!(plan.hits.len(), 1, "one hit for the message in two folders");
    let next: Vec<_> = plan.hits[0]
        .placements
        .iter()
        .map(|placement| {
            (
                placement.collection.as_str(),
                placement.next.clone().unwrap(),
            )
        })
        .collect();
    let want: BTreeSet<String> = ["\\Flagged", "work"].map(String::from).into();
    assert_eq!(next, [("Archive", want.clone()), ("INBOX", want.clone())]);

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let actions = plan.stage(&mut producer).unwrap();
    assert_eq!(actions.len(), 2, "one set-flags per placement");
    assert!(
        actions.iter().all(|action| action.source == "local"),
        "the only source syncing each collection"
    );
    assert_eq!(
        store_flags(dir.path(), "INBOX", seq),
        ["\\Seen"],
        "staged only"
    );
    drop(producer);

    PimdirStore::open(dir.path())
        .unwrap()
        .for_source("local")
        .drain()
        .unwrap();
    for collection in ["INBOX", "Archive"] {
        assert_eq!(
            store_flags(dir.path(), collection, seq),
            ["\\Flagged", "work"]
        );
    }

    let again = index
        .tag(&query, &["work".into()], &[], None, &clock)
        .unwrap();
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    assert!(
        again.stage(&mut producer).unwrap().is_empty(),
        "a placement the change leaves as it is stages nothing"
    );
}

#[test]
fn a_tag_refuses_a_change_naming_no_flag() {
    let dir = tempfile::tempdir().unwrap();
    let (index, _) = tag_store(dir.path());
    let query = PimdirQuery::parse("alpha").unwrap();
    let clock = PimdirSearchClock::default();
    let refusal = |add: &[&str], remove: &[&str]| {
        let add: Vec<String> = add.iter().map(|flag| flag.to_string()).collect();
        let remove: Vec<String> = remove.iter().map(|flag| flag.to_string()).collect();
        match index.tag(&query, &add, &remove, None, &clock) {
            Err(PimdirError::Tag(err)) => err,
            other => panic!("{add:?} {remove:?}: refused, got {other:?}"),
        }
    };

    assert_eq!(refusal(&[], &[]), PimdirSearchTagError::Empty);
    assert_eq!(
        refusal(&["unread"], &[]),
        PimdirSearchTagError::NotAFlag("unread".into())
    );
    assert_eq!(
        refusal(&["read"], &["\\Seen"]),
        PimdirSearchTagError::Both("\\Seen".into())
    );
}

#[test]
fn a_thread_answers_its_members_from_any_of_them() {
    let dir = tempfile::tempdir().unwrap();
    file_message(dir.path(), "INBOX", mail("p@x", 9, ""), &[]);
    let reply = mail("c@x", 10, "In-Reply-To: <p@x>\r\n");
    file_message(dir.path(), "INBOX", reply, &[]);
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");
    store
        .ensure_collection("Files", "application/octet-stream")
        .unwrap();
    let file = PimdirFileSummary {
        name: "notes.txt".into(),
        media_type: None,
        size: None,
        part: None,
    };
    store
        .put_file("Files", &PimdirLinkId("notes".into()), &file)
        .unwrap();
    drop(store);

    let mut index = PimdirIndex::open(dir.path()).unwrap();
    index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    let seq = |id: &str| hits(&index, &format!("id:{id}"))[0];
    let (root, child, notes) = (seq("p@x"), seq("c@x"), seq("notes"));

    for from in [child, root] {
        let thread = index.thread(None, from).unwrap().unwrap();
        let members: Vec<i64> = thread.hits.iter().map(|hit| hit.seq).collect();
        assert_eq!(
            (thread.thread.as_deref(), thread.count, members),
            (Some("p@x"), 2, vec![root, child]),
            "from {from}: oldest first"
        );
        assert_eq!(
            (thread.first.as_deref(), thread.last.as_deref()),
            (Some("2026-08-01T09:00:00Z"), Some("2026-08-01T10:00:00Z"))
        );
    }

    let alone = index.thread(None, notes).unwrap().unwrap();
    let members: Vec<i64> = alone.hits.iter().map(|hit| hit.seq).collect();
    assert_eq!((alone.thread, alone.count, members), (None, 1, vec![notes]));
    assert!(index.thread(Some("other"), root).unwrap().is_none());
}

/// Files a body into a collection of `kind` as `source`.
fn file_as(dir: &Path, source: &str, collection: &str, kind: &str, body: Vec<u8>) {
    let mut store = PimdirStore::open(dir).unwrap().for_source(source);
    store.ensure_collection(collection, kind).unwrap();
    let derived = summary::derive(kind, &body).unwrap();
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
            PimdirWriteOp::UpsertPlacement(placement(
                collection,
                derived.link_id.as_str(),
                derived.link_id.as_str(),
                Some(hash),
                derived.summary.unwrap(),
                derived.sort_key.as_str(),
                PimdirFlags::Known(BTreeSet::new()),
            )),
        ])
        .unwrap();
}

/// Declares `source` supporting every capability of `names` in full.
fn declare(dir: &Path, source: &str, names: &[&str]) {
    let capabilities: Vec<PimdirCapability> = names
        .iter()
        .map(|name| PimdirCapability {
            collection: None,
            name: name.to_string(),
            support: PimdirSupport::Full,
            detail: None,
        })
        .collect();
    PimdirStore::open(dir)
        .unwrap()
        .declare(source, &capabilities)
        .unwrap();
}

#[test]
fn a_tag_says_which_placements_reach_their_server() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    file_as(dir, "imap", "INBOX", "message/rfc822", mail("a@x", 9, ""));
    file_as(dir, "pop", "Local", "message/rfc822", mail("b@x", 10, ""));
    let event = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//x//x//EN\r\nBEGIN:VEVENT\r\n\
                 UID:standup@x\r\nDTSTAMP:20260801T000000Z\r\nDTSTART:20260801T090000Z\r\n\
                 SUMMARY:Standup\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    file_as(dir, "caldav", "Calendar", "text/calendar", event.into());
    declare(dir, "imap", capability::MAIL);
    declare(dir, "pop", &[capability::MAIL_FLAGS_SEEN]);

    let mut index = PimdirIndex::open(dir).unwrap();
    index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    let query = PimdirQuery::parse("kind:mail or kind:event").unwrap();
    let plan = index
        .tag(
            &query,
            &["work".into()],
            &[],
            None,
            &PimdirSearchClock::default(),
        )
        .unwrap();
    let reach = |plan: &PimdirSearchTagPlan| -> BTreeMap<String, (bool, Vec<String>)> {
        plan.hits
            .iter()
            .flat_map(|hit| &hit.placements)
            .map(|placement| {
                (
                    placement.collection.clone(),
                    (placement.syncs, placement.missing.clone()),
                )
            })
            .collect()
    };
    assert_eq!(
        reach(&plan),
        BTreeMap::from([
            ("Calendar".into(), (false, vec![])),
            ("INBOX".into(), (true, vec![])),
            (
                "Local".into(),
                (false, vec![capability::MAIL_FLAGS_KEYWORDS.to_string()])
            ),
        ]),
        "flags reach a server for mail alone, from a source declaring them"
    );

    let mut producer = PimdirProducer::open(dir, "test").unwrap();
    let staged: BTreeSet<String> = plan
        .stage(&mut producer)
        .unwrap()
        .into_iter()
        .map(|action| action.collection)
        .collect();
    assert_eq!(
        staged,
        BTreeSet::from(["Calendar".into(), "INBOX".into()]),
        "nothing for the source lacking the capability, the others staged"
    );
    drop(producer);
    PimdirStore::open(dir)
        .unwrap()
        .for_source("imap")
        .drain()
        .unwrap();
    let inbox = hits(&index, "id:a@x")[0];
    assert_eq!(store_flags(dir, "INBOX", inbox), ["work"]);
    let local = hits(&index, "id:b@x")[0];
    assert!(store_flags(dir, "Local", local).is_empty());

    // NOTE: a declaration changed between the plan and the stage: the
    // gate refuses at the stage, and the queue stays as it was.
    declare(dir, "pop", capability::MAIL);
    let plan = index
        .tag(
            &PimdirQuery::parse("id:b@x").unwrap(),
            &["late".into()],
            &[],
            None,
            &PimdirSearchClock::default(),
        )
        .unwrap();
    assert!(plan.hits[0].placements[0].missing.is_empty());
    declare(dir, "pop", &[capability::MAIL_FLAGS_SEEN]);
    let mut producer = PimdirProducer::open(dir, "test").unwrap();
    match plan.stage(&mut producer) {
        Err(PimdirError::Unsupported(refusal)) => assert_eq!(
            (refusal.source.as_str(), refusal.capability.as_str()),
            ("pop", capability::MAIL_FLAGS_KEYWORDS)
        ),
        other => panic!("the gate refuses the keyword on pop, got {other:?}"),
    }
    assert!(producer.pending_actions("Local").unwrap().is_empty());
}

/// A query client never builds, rebuilds nor refreshes the index (SEARCH
/// §3): a missing or outdated index is a coded error naming the indexer,
/// and an answer says how far the index lags the store.
#[test]
fn a_query_open_neither_builds_rebuilds_nor_refreshes() {
    let dir = tempfile::tempdir().unwrap();
    file_message(
        dir.path(),
        "INBOX",
        message("a@x", "Alpha", "first body"),
        &[],
    );

    let Err(err) = PimdirIndex::open_query(dir.path()) else {
        panic!("a missing index opens for queries");
    };
    assert_eq!(err.code(), "index-missing");
    assert!(!dir.path().join("index.db").exists(), "nothing created");

    let mut index = PimdirIndex::open(dir.path()).unwrap();
    index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    drop(index);

    let mut query = PimdirIndex::open_query(dir.path()).unwrap();
    assert_eq!(hits(&query, "alpha").len(), 1);
    let Err(err) = query.refresh(&PimdirIndexRefreshOptions::default()) else {
        panic!("a query handle refreshed");
    };
    assert_eq!(err.code(), "index-query-only");

    file_message(
        dir.path(),
        "INBOX",
        message("b@x", "Beta", "second body"),
        &[],
    );
    let answer = query
        .count(
            &PimdirQuery::parse("beta").unwrap(),
            &PimdirSearchClock::default(),
        )
        .unwrap();
    assert_eq!(answer.count, 0, "the index answers as it stands");
    assert!(answer.coverage.local.behind > 0, "and says it is behind");
    assert!(!answer.coverage.is_complete());
    drop(query);

    let conn = rusqlite::Connection::open(dir.path().join("index.db")).unwrap();
    conn.execute("UPDATE index_meta SET tokenizer = 'porter'", [])
        .unwrap();
    drop(conn);
    let Err(err) = PimdirIndex::open_query(dir.path()) else {
        panic!("an outdated index opens for queries");
    };
    assert_eq!(err.code(), "index-outdated");
    let conn = rusqlite::Connection::open(dir.path().join("index.db")).unwrap();
    let tokenizer: String = conn
        .query_row("SELECT tokenizer FROM index_meta", [], |r| r.get(0))
        .unwrap();
    assert_eq!(tokenizer, "porter", "nothing rebuilt");
}

/// An index an earlier draft built is told by its shape (SEARCH §2): one
/// whose tables, indexes or triggers differ from the schema's is rebuilt
/// by the indexer and refused by a query client, while a table of another
/// implementation's beside them, and FTS5's own, change nothing.
#[test]
fn an_index_of_another_shape_is_rebuilt_never_queried() {
    let dir = tempfile::tempdir().unwrap();
    file_message(
        dir.path(),
        "INBOX",
        message("a@x", "Alpha", "first body"),
        &[],
    );
    let mut index = PimdirIndex::open(dir.path()).unwrap();
    index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    drop(index);

    let conn = rusqlite::Connection::open(dir.path().join("index.db")).unwrap();
    conn.execute("CREATE TABLE another_indexer (x INTEGER)", [])
        .unwrap();
    drop(conn);
    let index = PimdirIndex::open_query(dir.path()).unwrap();
    assert_eq!(hits(&index, "alpha").len(), 1, "another table is left out");
    drop(index);

    let conn = rusqlite::Connection::open(dir.path().join("index.db")).unwrap();
    conn.execute("DROP INDEX placement_by_link", []).unwrap();
    drop(conn);
    let Err(err) = PimdirIndex::open_query(dir.path()) else {
        panic!("an index of another shape opens for queries");
    };
    assert_eq!(err.code(), "index-outdated");

    let mut index = PimdirIndex::open(dir.path()).unwrap();
    assert_eq!(index.cursor().unwrap(), 0, "rebuilt, not migrated");
    index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    drop(index);
    let index = PimdirIndex::open_query(dir.path()).unwrap();
    assert_eq!(hits(&index, "alpha").len(), 1);
}

/// A collection's name is a label, never an address (STORAGE §14):
/// `collection:` and `folder:` name an id alone.
#[test]
fn a_collection_term_names_an_id_never_a_display_name() {
    let dir = tempfile::tempdir().unwrap();
    file_message(
        dir.path(),
        "INBOX",
        message("a@x", "Alpha", "first body"),
        &[],
    );
    PimdirStore::open(dir.path())
        .unwrap()
        .set_collection_name("INBOX", "Archive")
        .unwrap();

    let mut index = PimdirIndex::open(dir.path()).unwrap();
    index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    assert_eq!(hits(&index, "collection:INBOX").len(), 1);
    assert_eq!(hits(&index, "collection:Archive"), Vec::<i64>::new());
    assert_eq!(hits(&index, "folder:Archive"), Vec::<i64>::new());
}

/// A rename a deadline-cut pass folded past is still reconciled by the
/// next pass completing (SEARCH §4): the cut pass advanced the cursor past
/// the collection's stamp, which no longer says anything moved.
#[test]
fn a_rename_a_cut_pass_went_past_is_reconciled_by_the_next_one() {
    let dir = tempfile::tempdir().unwrap();
    file_message(
        dir.path(),
        "INBOX",
        message("a@x", "Alpha", "first body"),
        &[],
    );
    file_message(
        dir.path(),
        "INBOX",
        message("b@x", "Beta", "second body"),
        &[],
    );

    let mut index = PimdirIndex::open(dir.path()).unwrap();
    index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    PimdirStore::open(dir.path())
        .unwrap()
        .rename_collection("INBOX", "Mail")
        .unwrap();

    // NOTE: the state a pass cut by its deadline right after the batch
    // folding the restamped items leaves: its cursor at their last stamp,
    // the purge count as the previous pass recorded it, the completing
    // transaction never run (SEARCH §4).
    let head = io_pimdir::client::reader::PimdirReader::open(dir.path())
        .unwrap()
        .change_cursor()
        .unwrap()
        .changed;
    let conn = rusqlite::Connection::open(dir.path().join("index.db")).unwrap();
    conn.execute("UPDATE index_meta SET store_change = ?1", [head])
        .unwrap();
    drop(conn);

    let report = index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    assert!(report.reconciled, "the rename is reconciled");
    assert_eq!(report.removed, 2, "the old id's placements go");
}

/// A value of digits names the item of that seq and every item whose link
/// id it is (SEARCH §8), a hit set rather than a guess.
#[test]
fn an_id_of_digits_names_the_seq_and_the_link_id() {
    let dir = tempfile::tempdir().unwrap();
    file_message(
        dir.path(),
        "INBOX",
        message("a@x", "Alpha", "first body"),
        &[],
    );
    file_message(
        dir.path(),
        "INBOX",
        message("777", "Digits", "second body"),
        &[],
    );

    let mut index = PimdirIndex::open(dir.path()).unwrap();
    index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    let alpha = hits(&index, "alpha");
    let digits = hits(&index, "subject:digits");
    assert_eq!((alpha.len(), digits.len()), (1, 1));

    assert_eq!(hits(&index, "id:777"), digits, "the link id 777");
    assert_eq!(hits(&index, &format!("id:{}", alpha[0])), alpha, "a seq");
    let mut both = hits(&index, &format!("id:{} or id:777", alpha[0]));
    both.sort();
    assert_eq!(both.len(), 2);
}
