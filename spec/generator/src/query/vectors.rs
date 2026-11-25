//! # The search vectors as a corpus
//!
//! vectors/search/store.json and each store of extraction.json rendered
//! into the model as the generator renders any store (SEARCH §11): each
//! fixture filed as it stands with the row vectors/summaries.json pins, a
//! bodiless item from the summary it states, the references and properties
//! as listed, an item stating `unreadable` counted missing; and each case
//! of queries.json and of extraction.json held to what the oracle decides
//! of it, a cross-check of the oracle by the vectors. refresh.json is not:
//! the model does not follow its owner writes.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

use rusqlite::types::Value;
use serde_json::Value as Json;

use crate::{
    body::Filed,
    error::{Error, Result},
    fixture,
    model::{
        Address, Collection, Kind, Model, Placement, Presentation, Reference, Scope, SourceState,
        Summary, Table,
    },
    query::{
        Clock, Corpus, Counts, Hit, Matched, Order, Property, Readings, Temporal, date::Zone,
        grammar,
    },
    spec::Spec,
    time::Instant,
};

/// What the oracle made of one case of queries.json.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Everything the case states, the oracle decides alike.
    Agreed,
    /// What the oracle decides agrees; some of the case it cannot decide.
    Partly(String),
    /// The oracle decides nothing the case states.
    Undecided(String),
    /// The oracle decides otherwise.
    Disagreed(String),
}

/// The store of store.json as a corpus, its index's horizon the one
/// queries.json states, and the instant its dates resolve against.
pub fn corpus(spec: &Spec) -> Result<(Corpus, i64)> {
    let store = read(spec, "vectors/search/store.json")?;
    let queries = read(spec, "vectors/search/queries.json")?;

    let instant =
        |key: &str| Instant::from_rfc3339(queries["horizon"][key].as_str().unwrap_or_default()).0;
    let corpus = render(spec, &store, (instant("start"), instant("end")))?;
    let now = Instant::from_rfc3339(queries["now"].as_str().unwrap_or_default()).0;
    Ok((corpus, now))
}

/// A store as the search vectors state one, rendered as a corpus whose
/// index holds `horizon`.
fn render(spec: &Spec, store: &Json, horizon: (i64, i64)) -> Result<Corpus> {
    let fixtures = fixture::load(spec)?;

    let mut model = Model::new();
    for collection in store["collections"].as_array().into_iter().flatten() {
        let id = text(&collection["id"]);
        let sources = collection["sources"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|source| {
                let instant = |key: &str| source[key].as_str().map(Instant::from_rfc3339);
                let coverage = source["covered_at"].as_str().map(|_| Scope {
                    since: instant("covered_since"),
                    until: instant("covered_until"),
                });
                let state = SourceState {
                    coverage,
                    ..SourceState::default()
                };
                (text(&source["source"]), state)
            })
            .collect();
        model.declare(Collection {
            id: id.clone(),
            account: collection["account"].as_str().map(String::from),
            kind: kind(collection["kind"].as_str().unwrap_or_default()),
            name: id,
            role: collection["role"].as_str().map(String::from),
            sources,
            conflict: String::from("manual"),
            generation: 1,
            parent: None,
            presentation: Presentation::default(),
        });
    }

    let mut missing = BTreeSet::new();
    let mut opaque = BTreeSet::new();
    for item in store["items"].as_array().into_iter().flatten() {
        let collection = text(&item["collection"]);
        let seq = item["seq"].as_i64().unwrap_or_default();
        let (filed, held) = match item["fixture"].as_str() {
            Some(path) => match fixtures.iter().find(|fixture| fixture.path == path) {
                Some(fixture) => (fixture.filed.clone(), true),
                None => {
                    opaque.insert((collection.clone(), seq));
                    (
                        unknown(spec, path, model.collections[&collection].kind)?,
                        true,
                    )
                }
            },
            None => (bodiless(&item["summary"]), false),
        };
        if item["unreadable"].as_bool() == Some(true) {
            missing.insert((collection.clone(), seq));
        }
        let flags = item["flags"]
            .as_array()
            .map(|flags| flags.iter().map(text).collect());
        let placement = Placement {
            flags,
            held,
            handle: None,
            revision: None,
            source: None,
        };
        let link_id = text(&item["link_id"]);
        model.place(&collection, link_id.clone(), seq, link_id, filed, placement);
        model.next_seq = model.next_seq.max(seq + 1);
    }

    for reference in store["references"].as_array().into_iter().flatten() {
        let end =
            |key: &str| kind(reference[key].as_str().unwrap_or_default()).unwrap_or(Kind::File);
        model.references.push(Reference {
            from_kind: end("from_kind"),
            from_link_id: text(&reference["from_link_id"]),
            to_kind: end("to_kind"),
            to_link_id: text(&reference["to_link_id"]),
            role: text(&reference["role"]),
            origin: text(&reference["origin"]),
        });
    }

    let properties = store["properties"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|property| Property {
            seq: property["seq"].as_i64().unwrap_or_default(),
            key: text(&property["key"]),
            value: text(&property["value"]),
        })
        .collect();

    Ok(Corpus {
        model,
        properties,
        missing,
        opaque,
        horizon,
        floating: Zone::Utc,
        readings: Readings::default(),
    })
}

/// Holds every case of queries.json to what the oracle decides of it,
/// answering each case's query with the oracle's verdict.
pub fn check(spec: &Spec) -> Result<Vec<(String, Verdict)>> {
    let (corpus, now) = corpus(spec)?;
    let queries = read(spec, "vectors/search/queries.json")?;

    let mut verdicts = Vec::new();
    for case in queries["cases"].as_array().into_iter().flatten() {
        let query = case["query"].as_str().unwrap_or_default();
        let verdict = verdict(&corpus, now, case);
        verdicts.push((String::from(query), verdict));
    }
    Ok(verdicts)
}

/// Holds every query of extraction.json to what the oracle decides of it
/// over its case's store, indexed at the file's `now` under SEARCH §7's
/// default horizon, a year back and two forward.
pub fn check_extraction(spec: &Spec) -> Result<Vec<(String, Verdict)>> {
    let extraction = read(spec, "vectors/search/extraction.json")?;
    let now = Instant::from_rfc3339(extraction["now"].as_str().unwrap_or_default()).0;
    let (year, month, day, hour, minute, second) = Instant(now).parts();
    let at = |year: i64| Instant::at(year, month, day, hour, minute, second).0;
    let horizon = (at(year - 1), at(year + 2));

    let mut verdicts = Vec::new();
    for case in extraction["cases"].as_array().into_iter().flatten() {
        let label = case["label"].as_str().unwrap_or_default();
        let corpus = render(spec, &case["store"], horizon)?;
        for query in case["queries"].as_array().into_iter().flatten() {
            let text = query["query"].as_str().unwrap_or_default();
            let verdict = verdict(&corpus, now, query);
            verdicts.push((format!("{label}: {text}"), verdict));
        }
    }
    Ok(verdicts)
}

fn verdict(corpus: &Corpus, now: i64, case: &Json) -> Verdict {
    let query = case["query"].as_str().unwrap_or_default();

    let parsed = grammar::parse(query);
    if let Some(error) = case.get("error") {
        let code = error["code"].as_str().unwrap_or_default();
        let span = (error["start"].as_u64(), error["end"].as_u64());
        return match parsed {
            Err(refusal)
                if refusal.code == code
                    && span == (Some(refusal.start as u64), Some(refusal.end as u64)) =>
            {
                Verdict::Agreed
            }
            Err(refusal) => Verdict::Disagreed(format!(
                "refused {} {}-{}, the case {code} {:?}-{:?}",
                refusal.code, refusal.start, refusal.end, span.0, span.1
            )),
            Ok(_) => Verdict::Disagreed(format!("answered, the case refuses {code}")),
        };
    }
    let node = match parsed {
        Ok(node) => node,
        Err(refusal) => return Verdict::Disagreed(format!("refused {}", refusal.code)),
    };
    if case.get("threads").is_some() {
        return Verdict::Undecided(String::from("a threaded listing"));
    }

    let zone = case["zone"].as_str().map(Zone::named);
    let index_zone = case["index_zone"]
        .as_str()
        .or(case["zone"].as_str())
        .map(Zone::named);
    let (Some(zone), Some(floating)) = (
        zone.unwrap_or(Some(Zone::Utc)),
        index_zone.unwrap_or(Some(Zone::Utc)),
    ) else {
        return Verdict::Undecided(String::from("a zone the model holds no rule for"));
    };
    let clock = Clock {
        now: case["now"]
            .as_str()
            .map_or(now, |now| Instant::from_rfc3339(now).0),
        zone,
    };
    let order = match case["sort"].as_str() {
        None | Some("newest-first") => Order::NewestFirst,
        Some("oldest-first") => Order::OldestFirst,
        Some("title") => Order::Title,
        Some("sender") => Order::Sender,
        Some(_) => Order::Relevance,
    };
    let corpus = Corpus {
        floating,
        ..corpus.clone()
    };
    let answer = corpus.answer(&node, &clock, order);

    let want: BTreeSet<Hit> = pairs(&case["hits"]).into_iter().collect();
    let mut differences = Vec::new();
    let mut undecided = Vec::new();

    let (yes, unknown) = answer.split();
    match unknown.is_empty() {
        true if yes != want => differences.push(format!("hits {yes:?}, the case {want:?}")),
        true => {}
        false => {
            let wrong: Vec<&Hit> = yes.iter().filter(|hit| !want.contains(hit)).collect();
            let lost: Vec<&Hit> = want
                .iter()
                .filter(|hit| !yes.contains(hit) && !unknown.contains(hit))
                .collect();
            if !wrong.is_empty() || !lost.is_empty() {
                differences.push(format!(
                    "hits: the oracle adds {wrong:?} and drops {lost:?}"
                ));
            }
            undecided.push(format!("{} items undecided", unknown.len()));
        }
    }

    if case["ordered"].as_bool() == Some(true) {
        match &answer.order {
            Some(order) if *order != pairs(&case["hits"]) => {
                differences.push(format!(
                    "order {order:?}, the case {:?}",
                    pairs(&case["hits"])
                ));
            }
            Some(_) => {}
            None => undecided.push(String::from("the order")),
        }
    }

    let coverage = &answer.coverage;
    if let Some(stated) = case.get("coverage") {
        let stated = counts(stated);
        match &coverage.kinds {
            Some(kinds) if *kinds != stated => {
                differences.push(format!("coverage {kinds:?}, the case {stated:?}"))
            }
            Some(_) => {}
            None => undecided.push(String::from("the coverage")),
        }
    }
    let mut part =
        |name: &str, stated: Option<String>, oracle: Option<String>| match (stated, oracle) {
            (Some(stated), Some(oracle)) if stated != oracle => {
                differences.push(format!("{name} {oracle}, the case {stated}"));
            }
            (Some(_), None) => undecided.push(String::from(name)),
            _ => {}
        };
    part(
        "complete",
        case["complete"]
            .as_bool()
            .map(|complete| complete.to_string()),
        coverage.complete.map(|complete| complete.to_string()),
    );
    part(
        "horizon_left",
        Some(case["horizon_left"].as_bool().unwrap_or(false).to_string()),
        coverage.horizon_left.map(|left| left.to_string()),
    );
    part(
        "zone_differs",
        Some(case["zone_differs"].as_bool().unwrap_or(false).to_string()),
        coverage.zone_differs.map(|differs| differs.to_string()),
    );
    part(
        "cut",
        case["cut"].as_u64().map(|cut| cut.to_string()),
        coverage.cut.map(|cut| cut.to_string()),
    );
    part(
        "temporal",
        case.get("temporal")
            .map(|temporal| format!("{:?}", temporal_of(temporal))),
        coverage
            .temporal
            .as_ref()
            .map(|temporal| format!("{temporal:?}")),
    );

    for stated in case["matched"].as_array().into_iter().flatten() {
        let hit = Hit {
            account: stated[0]
                .as_str()
                .filter(|account| !account.is_empty())
                .map(String::from),
            seq: stated[1].as_i64().unwrap_or_default(),
        };
        let stated = stated[2].as_str().unwrap_or_default();
        let oracle = match answer.matched.get(&hit) {
            Some(Some(Matched::Body)) => "body",
            Some(Some(Matched::Summary)) => "summary",
            Some(None) => "nothing",
            None => {
                undecided.push(format!("what {hit:?} matched"));
                continue;
            }
        };
        if oracle != stated {
            differences.push(format!("{hit:?} matched {oracle}, the case {stated}"));
        }
    }

    match (differences.is_empty(), undecided.is_empty()) {
        (false, _) => Verdict::Disagreed(differences.join("; ")),
        (true, true) => Verdict::Agreed,
        (true, false) if yes.is_empty() && !unknown.is_empty() && !want.is_empty() => {
            Verdict::Undecided(undecided.join(", "))
        }
        (true, false) => Verdict::Partly(undecided.join(", ")),
    }
}

fn temporal_of(temporal: &Json) -> Temporal {
    match temporal.as_str() {
        Some(_) => Temporal::Full,
        None => Temporal::Bounded {
            since: temporal["since"].as_str().map(String::from),
            until: temporal["until"].as_str().map(String::from),
            filling: temporal["filling"].as_bool().unwrap_or(false),
        },
    }
}

fn counts(coverage: &Json) -> BTreeMap<String, Counts> {
    coverage
        .as_object()
        .into_iter()
        .flatten()
        .map(|(kind, row)| {
            let count = |key: &str| row[key].as_u64().unwrap_or_default();
            let counts = Counts {
                bodiless: count("bodiless"),
                missing: count("missing"),
                unparseable: count("unparseable"),
                encrypted: count("encrypted"),
            };
            (kind.clone(), counts)
        })
        .collect()
}

fn pairs(hits: &Json) -> Vec<Hit> {
    hits.as_array()
        .into_iter()
        .flatten()
        .map(|pair| Hit {
            account: pair[0]
                .as_str()
                .filter(|account| !account.is_empty())
                .map(String::from),
            seq: pair[1].as_i64().unwrap_or_default(),
        })
        .collect()
}

fn bodiless(summary: &Json) -> Filed {
    let table = match summary["table"].as_str() {
        Some("mail_summary") => Table::Mail,
        Some("contact_summary") => Table::Contact,
        Some("event_summary") => Table::Event,
        Some("task_summary") => Table::Task,
        Some("journal_summary") => Table::Journal,
        _ => Table::File,
    };
    let mut row = Summary::new(table);
    for (column, value) in summary["row"].as_object().into_iter().flatten() {
        match column.as_str() {
            "in_reply_to" => row.in_reply_to = strings(value),
            "categories" => row.categories = strings(value),
            column => row.set(column, sql(value)),
        }
    }
    let addresses = summary["addresses"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|address| Address {
            role: text(&address["role"]),
            position: address["position"].as_i64().unwrap_or_default(),
            address: text(&address["address"]),
            name: address["name"].as_str().map(String::from),
        })
        .collect();
    Filed {
        bytes: Vec::new(),
        hint: None,
        summary: row,
        addresses,
        sort_key: text(&summary["sort_key"]),
        fallback: None,
        words: None,
    }
}

/// A fixture summaries.json pins no row for: its bytes, and a summary the
/// model does not know, which the corpus holds opaque.
fn unknown(spec: &Spec, path: &str, kind: Option<Kind>) -> Result<Filed> {
    let body = spec.root.join("vectors").join(path);
    let bytes = fs::read(&body).map_err(|source| Error::Io { path: body, source })?;
    let table = match kind {
        Some(Kind::Mail) => Table::Mail,
        Some(Kind::Contact) => Table::Contact,
        Some(Kind::Calendar) => Table::Event,
        _ => Table::File,
    };
    Ok(Filed {
        bytes,
        hint: None,
        summary: Summary::new(table),
        addresses: Vec::new(),
        sort_key: String::new(),
        fallback: None,
        words: None,
    })
}

fn kind(media_type: &str) -> Option<Kind> {
    match media_type {
        "message/rfc822" => Some(Kind::Mail),
        "text/vcard" => Some(Kind::Contact),
        "text/calendar" => Some(Kind::Calendar),
        "application/octet-stream" => Some(Kind::File),
        _ => None,
    }
}

fn text(value: &Json) -> String {
    String::from(value.as_str().unwrap_or_default())
}

fn strings(value: &Json) -> Vec<String> {
    value.as_array().into_iter().flatten().map(text).collect()
}

fn sql(value: &Json) -> Value {
    match value {
        Json::Null => Value::Null,
        Json::Bool(value) => Value::Integer(i64::from(*value)),
        Json::Number(number) => number.as_i64().map_or(Value::Null, Value::Integer),
        Json::String(text) => Value::Text(text.clone()),
        other => Value::Text(other.to_string()),
    }
}

fn read(spec: &Spec, path: &str) -> Result<Json> {
    let path = spec.root.join(path);
    let text = fs::read_to_string(&path).map_err(|source| Error::Io {
        path: path.clone(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|source| Error::Json { path, source })
}

#[cfg(test)]
mod tests {
    use crate::{
        query::vectors::{Verdict, check, check_extraction},
        spec::Spec,
    };

    #[test]
    fn the_oracle_answers_the_search_vectors_alike_where_it_decides() {
        let spec = Spec::load_default().unwrap();
        let mut verdicts = check(&spec).unwrap();
        verdicts.extend(check_extraction(&spec).unwrap());
        for (query, verdict) in &verdicts {
            println!("{query:?}: {verdict:?}");
        }
        let disagreed: Vec<_> = verdicts
            .iter()
            .filter(|(_, verdict)| matches!(verdict, Verdict::Disagreed(_)))
            .collect();
        assert!(disagreed.is_empty(), "{disagreed:#?}");
    }
}
