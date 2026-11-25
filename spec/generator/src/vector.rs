//! # Sync vectors
//!
//! A [`Case`] written as one case of the pimdir standard's vectors/sync/
//! (format 11, vectors/README.md): the store before the run as rows, the
//! run, the remote's pages, fetches and outcomes, and what the model
//! states after it, every body a fixture under vectors/fixtures/ named by
//! its object name. A case the format holds no field for (a head, a run
//! cut after a push it made, a mutation) is refused, to be reported with
//! its history instead (README.md, "Shrinking").

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::types::Value;
use serde_json::{Map, Value as Json, json};

use crate::{
    engine::{Answer, Event, Push},
    model::{Item, Model, State, Table},
    name::ObjectName,
    shrink::Case,
};

/// The format version written.
pub const FORMAT: u64 = 11;

/// What the run was asked, which the [`Case`]'s run does not state
/// (finding G11 of io-pimdir's replay): its options as the replay chose
/// them and the connector's own answers.
#[derive(Clone, Debug)]
pub struct Asked {
    /// The run's conflict policy as vectors/sync spells it: `manual`,
    /// `prefer-remote`, `prefer-local` or `keep-local`.
    pub conflict: String,
    /// The scope's floor, RFC 3339, if any.
    pub since: Option<String>,
    /// The scope's ceiling, RFC 3339, if any.
    pub until: Option<String>,
    /// Whether the run accepts absences.
    pub accept_absences: bool,
    /// Whether the connector's checkpoint is bound to its scope.
    pub scope_bound: bool,
    /// Whether the connector rejects the cursor the run resumes from.
    pub cursor_rejected: bool,
}

/// A written case and the fixtures it names, by file name.
#[derive(Clone, Debug)]
pub struct Vector {
    /// The case.
    pub json: Json,
    /// Each fixture's file name under vectors/fixtures/ and its bytes.
    pub fixtures: BTreeMap<String, Vec<u8>>,
}

/// The labels of the case's bodies and the fixtures holding them.
#[derive(Default)]
struct Bodies {
    labels: BTreeMap<ObjectName, String>,
    objects: Map<String, Json>,
    fixtures: BTreeMap<String, Vec<u8>>,
}

impl Bodies {
    /// The fixture path `bytes` are written to, under vectors/.
    fn fixture(&mut self, bytes: &[u8]) -> String {
        let extension = match bytes {
            [
                b'B',
                b'E',
                b'G',
                b'I',
                b'N',
                b':',
                b'V',
                b'C',
                b'A',
                b'R',
                b'D',
                ..,
            ] => "vcf",
            [
                b'B',
                b'E',
                b'G',
                b'I',
                b'N',
                b':',
                b'V',
                b'C',
                b'A',
                b'L',
                ..,
            ] => "ics",
            _ => "eml",
        };
        let file = format!("generated-{}.{extension}", ObjectName::of(bytes).0);
        self.fixtures.insert(file.clone(), bytes.to_vec());
        format!("fixtures/{file}")
    }

    /// The label of the body `name`, its bytes found in `models`.
    fn label(&mut self, name: &ObjectName, models: &[&Model]) -> Json {
        if let Some(label) = self.labels.get(name) {
            return json!(label);
        }
        let bytes = models
            .iter()
            .find_map(|model| model.objects.get(name))
            .cloned()
            .unwrap_or_default();
        self.label_bytes(&bytes)
    }

    fn label_bytes(&mut self, bytes: &[u8]) -> Json {
        let name = ObjectName::of(bytes);
        if let Some(label) = self.labels.get(&name) {
            return json!(label);
        }
        let label = format!("o{}", self.labels.len() + 1);
        let body = self.fixture(bytes);
        self.objects.insert(label.clone(), json!({ "body": body }));
        self.labels.insert(name, label.clone());
        json!(label)
    }

    fn optional(&mut self, name: &Option<ObjectName>, models: &[&Model]) -> Json {
        match name {
            Some(name) => self.label(name, models),
            None => Json::Null,
        }
    }
}

/// Writes `case` as a sync vector labelled `label` with `note`, the
/// summary columns kept to `columns`, those of STORAGE §4.3's `summary`
/// table.
pub fn write(
    case: &Case,
    asked: &Asked,
    label: &str,
    note: &str,
    columns: &BTreeSet<String>,
) -> Result<Vector, String> {
    let run = &case.run;
    if run.head.is_some() {
        return Err(String::from("the run lists a head"));
    }
    if run.failed && !run.pushes.is_empty() {
        return Err(String::from("the run is cut after a push"));
    }
    if !matches!(run.verb, "sync" | "upgrade" | "rekey") {
        return Err(format!("the format states no generated {}", run.verb));
    }
    let models = [&case.before, &case.after];
    let mut bodies = Bodies::default();

    let mut before = rows(&case.before, &mut bodies, &models, columns);
    let after = rows(&case.after, &mut bodies, &models, columns);

    let page = |page: &crate::remote::Page, bodies: &mut Bodies| {
        let items: Vec<Json> = page
            .items
            .iter()
            .map(|item| {
                bodies.label_bytes(&item.meta);
                let body = bodies.fixture(&item.meta);
                let mut member = json!({
                    "handle": item.handle,
                    "flags": item.flags,
                    "revision": item.revision,
                    "meta": { "body": body },
                });
                if item.carries {
                    member["carries"] = json!(true);
                }
                member
            })
            .collect();
        let mut page_json = json!({
            "items": items,
            "vanished": page.vanished,
            "complete": page.complete,
            "last": page.last,
            "cursor": page.cursor,
            "checkpoint": page.checkpoint,
        });
        if page.narrowed {
            page_json["narrowed"] = json!(true);
        }
        if page.capped {
            page_json["capped"] = json!(true);
        }
        if let Some(enumerated) = &page.enumerated {
            page_json["enumerated"] = json!(enumerated);
        }
        page_json
    };
    let pages: Vec<Json> = run.pages.iter().map(|p| page(p, &mut bodies)).collect();

    let mut fetch = Map::new();
    for (handle, bytes) in &run.fetch {
        bodies.label_bytes(bytes);
        let body = bodies.fixture(bytes);
        let revision = case
            .revisions
            .get(&(run.collection.clone(), handle.clone()))
            .cloned();
        let answer = json!({ "body": body, "revision": revision });
        fetch.insert(
            handle.clone(),
            json!({ "Meta": answer.clone(), "Full": answer }),
        );
    }

    let pushes: Vec<Json> = run
        .pushes
        .iter()
        .map(|(push, key)| match push {
            Push::Add {
                handle,
                link_id,
                flags,
                origin,
                object,
            } => json!({
                "kind": "Add",
                "handle": handle,
                "key": key,
                "link_id": link_id,
                "flags": flags,
                "origin": origin.as_ref().map(|(collection, handle)| json!({
                    "collection": collection,
                    "handle": handle,
                })),
                "object": bodies.label(object, &models),
            }),
            Push::Remove {
                handle,
                to,
                link_id,
                if_match,
            } => json!({
                "kind": "Remove",
                "handle": handle,
                "key": key,
                "to": to,
                "link_id": link_id,
                "if_match": if_match,
            }),
            Push::SetFlags {
                handle,
                add,
                remove,
            } => json!({
                "kind": "SetFlags",
                "handle": handle,
                "key": key,
                "add": add,
                "remove": remove,
            }),
            Push::Update {
                handle,
                object,
                if_match,
            } => json!({
                "kind": "Update",
                "handle": handle,
                "key": key,
                "object": bodies.label(object, &models),
                "if_match": if_match,
            }),
        })
        .collect();

    let outcomes: Vec<Json> = run
        .outcomes
        .iter()
        .map(|outcome| {
            let mut stated = json!({
                "handle": outcome.handle,
                "outcome": match &outcome.answer {
                    Answer::Accepted => "Accepted",
                    Answer::Rejected => "Rejected",
                    Answer::Refused(_) => "Refused",
                },
                "assigned": outcome.assigned,
                "revision": outcome.revision,
            });
            if let Answer::Refused(reason) = &outcome.answer {
                stated["reason"] = json!(reason);
            }
            if let Some(relocated) = outcome.relocated {
                stated["relocated"] = json!(relocated);
            }
            stated
        })
        .collect();

    let events: Vec<Json> = run.events.iter().map(event).collect();

    before["objects"] = Json::Object(bodies.objects.clone());
    let mut options = json!({
        "push": true,
        "rights": { "flags": true, "content": true, "add": true, "remove": true },
        "conflict": asked.conflict,
        "accept_absences": asked.accept_absences,
    });
    if asked.since.is_some() || asked.until.is_some() {
        options["scope"] = json!({ "since": asked.since, "until": asked.until });
    }
    let mut run_json = json!({
        "verb": run.verb,
        "collection": run.collection,
        "source": run.source,
        "options": options,
    });
    if run.verb == "upgrade" {
        run_json["tier"] = json!("Full");
        run_json["handles"] = json!(run.handles);
    }

    let mut remote = json!({ "pages": pages, "fetch": fetch });
    if !asked.scope_bound {
        remote["scope_bound"] = json!(false);
    }
    if asked.cursor_rejected {
        remote["cursor_rejected"] = json!(true);
    }
    if run.failed {
        remote["interrupted_after"] = json!(run.pages.len());
    }

    let mut expect = json!({
        "pushes": pushes,
        "outcomes": outcomes,
        "events": events,
        "store": {
            "items": after["items"],
            "bindings": after["bindings"],
            "sources": after["sources"],
            "summaries": after["summaries"],
        },
    });
    if let Some((absent, bound)) = run.parked {
        expect["parked"] = json!({ "absent": absent, "bound": bound });
    }
    if let Some(unlisted) = run.unlisted {
        expect["unlisted"] = json!(unlisted);
    }
    if run.failed {
        expect["failed"] = json!(true);
    }

    Ok(Vector {
        json: json!({
            "v": FORMAT,
            "label": label,
            "note": note,
            "store": before,
            "run": run_json,
            "remote": remote,
            "expect": expect,
        }),
        fixtures: bodies.fixtures,
    })
}

/// The rows of `model` as a case's `store` states them.
fn rows(model: &Model, bodies: &mut Bodies, models: &[&Model], columns: &BTreeSet<String>) -> Json {
    let collections: Vec<Json> = model
        .collections
        .values()
        .map(|collection| {
            json!({
                "id": collection.id,
                "account": collection.account,
                "kind": collection.kind.map_or("", |kind| kind.media_type()),
                "conflict": collection.conflict,
                "generation": collection.generation,
            })
        })
        .collect();

    let mut items = Vec::new();
    let mut bindings = Vec::new();
    let mut summaries = Vec::new();
    for item in &model.items {
        items.push(item_row(item, bodies, models));
        for binding in &item.bindings {
            bindings.push(json!({
                "collection": item.collection,
                "link_id": item.link_id,
                "source": binding.source,
                "handle": binding.handle,
                "base_flags": binding.base_flags,
                "base_object": bodies.optional(&binding.base_object, models),
                "base_revision": binding.base_revision,
                "base_present": i64::from(binding.base_present),
                "conflicted": i64::from(binding.conflicted),
                "conflict_revision": binding.conflict_revision,
                "conflict_object": bodies.optional(&binding.conflict_object, models),
                "shared_object": bodies.optional(&binding.shared_object, models),
                "refusal": binding.refusal.as_ref().map(|refusal| refusal.reason.clone()),
                "refused_key": binding.refusal.as_ref().map(|refusal| refusal.key.clone()),
            }));
        }
        if let Some(summary) = summary_row(item, columns) {
            summaries.push(summary);
        }
    }

    let mut sources = Vec::new();
    for collection in model.collections.values() {
        for (source, state) in &collection.sources {
            let text = |bytes: &Option<Vec<u8>>| {
                bytes
                    .as_ref()
                    .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            };
            let bound = |scope: Option<&crate::model::Scope>| {
                let (since, until) =
                    scope.map_or((Value::Null, Value::Null), |scope| scope.values());
                (cell(&since), cell(&until))
            };
            let (covered_since, covered_until) = bound(state.coverage.as_ref());
            let (round_since, round_until) = bound(state.open.as_ref().map(|open| &open.scope));
            sources.push(json!({
                "collection": collection.id,
                "source": source,
                "checkpoint": text(&state.checkpoint),
                "round": state.rounds,
                "round_open": state.open.is_some(),
                "round_since": round_since,
                "round_until": round_until,
                "round_cursor": state.open.as_ref().and_then(|open| open.cursor.clone()),
                "round_checkpoint": state.open.as_ref().and_then(|open| text(&open.checkpoint)),
                "round_band": state.open.as_ref().is_some_and(|open| open.band),
                "covered": state.coverage.is_some(),
                "covered_since": covered_since,
                "covered_until": covered_until,
            }));
        }
    }

    json!({
        "collections": collections,
        "items": items,
        "bindings": bindings,
        "summaries": summaries,
        "sources": sources,
    })
}

fn item_row(item: &Item, bodies: &mut Bodies, models: &[&Model]) -> Json {
    let (deleted, retained, retained_by) = match &item.state {
        State::Live => (0, false, None),
        State::Tombstone => (1, false, None),
        State::Retained { by } => (1, true, by.clone()),
    };
    json!({
        "collection": item.collection,
        "link_id": item.link_id,
        "seq": item.seq,
        "flags": item.flags,
        "object": bodies.optional(&item.object, models),
        "sort_key": item.sort_key,
        "level": item.level,
        "deleted": deleted,
        "retained": retained,
        "retained_by": retained_by,
        "conflicted": i64::from(item.conflicted),
        "conflict_object": bodies.optional(&item.conflict_object, models),
    })
}

/// An item's summary as a case's `summaries` states it, its columns those
/// the `summary` table holds.
fn summary_row(item: &Item, columns: &BTreeSet<String>) -> Option<Json> {
    let summary = &item.summary;
    if summary.columns.is_empty() {
        return None;
    }
    let table = summary.table.stem();
    let mut row = Map::new();
    for (name, value) in &summary.columns {
        let column = match (summary.table, name.as_str()) {
            (Table::Mail, "message_id") => "uid",
            (Table::Mail, "subject")
            | (Table::Contact, "fn")
            | (Table::File, "name")
            | (Table::Event | Table::Task | Table::Journal, "summary") => "title",
            (Table::Contact, "kind") => "card_kind",
            (_, column) => column,
        };
        if columns.contains(column) {
            row.insert(name.clone(), cell(value));
        }
    }
    match summary.table {
        Table::Mail => {
            row.insert(String::from("in_reply_to"), json!(summary.in_reply_to));
        }
        Table::Task => {
            row.insert(String::from("categories"), json!(summary.categories));
        }
        _ => {}
    }
    Some(json!({
        "table": format!("{table}_summary"),
        "collection": item.collection,
        "link_id": item.link_id,
        "row": row,
    }))
}

fn cell(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Integer(integer) => json!(integer),
        Value::Real(real) => json!(real),
        Value::Text(text) => json!(text),
        Value::Blob(_) => Json::Null,
    }
}

fn event(event: &Event) -> Json {
    match event {
        Event::Added(handle) => json!({ "kind": "Added", "handle": handle }),
        Event::FlagsChanged(handle) => json!({ "kind": "FlagsChanged", "handle": handle }),
        Event::ContentChanged(handle) => json!({ "kind": "ContentChanged", "handle": handle }),
        Event::Conflicted(handle) => json!({ "kind": "Conflicted", "handle": handle }),
        Event::Vanished(handle) => json!({ "kind": "Vanished", "handle": handle }),
        Event::Created(handle) => json!({ "kind": "Created", "handle": handle }),
        Event::Overruled { handle, link_id } => {
            json!({ "kind": "Overruled", "handle": handle, "link_id": link_id })
        }
        Event::Refused { handle, reason } => {
            json!({ "kind": "Refused", "handle": handle, "reason": reason })
        }
    }
}
