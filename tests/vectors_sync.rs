//! The sync conformance suite (pimdir SYNC.md §11): every case under the
//! specification's vectors/sync/ is built into a real store, run through
//! the engine against a scripted remote, and compared as parsed rows.
//!
//! The spec is a sibling checkout, so the suite skips when it is absent.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use io_pimdir::{
    change::{PimdirChange, PimdirChangeKind},
    client::{PimdirSourceStore, PimdirStore},
    collection::{PimdirCheckpoint, PimdirCollectionId},
    hash::PimdirHashAlgo,
    mutate::PimdirMutation,
    object::{PimdirHash, PimdirObject},
    placement::{PimdirFlags, PimdirHandle, PimdirLinkId},
    remote::{
        PimdirFetchedBody, PimdirFetchedItem, PimdirPushOutcome, PimdirPushResult, PimdirRemote,
        PimdirRemoteItem, PimdirRemoteSnapshot, PimdirTier,
    },
    sql,
    summary::{self, PimdirSummary},
    sync::{
        PimdirConflictPolicy, PimdirDeletePolicy, PimdirPushRights, PimdirSyncEvent,
        PimdirSyncOptions,
    },
};
use rusqlite::{Connection, named_params, params};
use serde_json::{Map, Value, json};

fn spec_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .join("pimdir");
    dir.join("vectors/sync").is_dir().then_some(dir)
}

/// Bodies by label, the store rows and the pushes name them by.
#[derive(Clone)]
struct Bodies {
    hashes: BTreeMap<String, String>,
    bytes: BTreeMap<String, Vec<u8>>,
}

impl Bodies {
    fn label(&self, hash: Option<String>) -> Value {
        match hash {
            None => Value::Null,
            Some(hash) => self
                .hashes
                .iter()
                .find(|(_, h)| **h == hash)
                .map(|(label, _)| json!(label))
                .unwrap_or(json!(hash)),
        }
    }

    fn hash(&self, label: &Value) -> Option<String> {
        label.as_str().map(|label| {
            self.hashes
                .get(label)
                .cloned()
                .unwrap_or_else(|| label.to_string())
        })
    }

    /// The object and bytes a mutation carries for a labelled body.
    fn object(&self, label: &Value) -> (PimdirObject, Vec<u8>) {
        let label = label.as_str().expect("a body label");
        let bytes = self.bytes[label].clone();
        let object = PimdirObject {
            hash: PimdirHash::from(self.hashes[label].clone()),
            size: bytes.len(),
        };
        (object, bytes)
    }
}

fn flags(value: &Value) -> PimdirFlags {
    match value.as_array() {
        Some(items) => PimdirFlags::from_iter(items.iter().filter_map(Value::as_str)),
        None => PimdirFlags::Unknown,
    }
}

fn flags_json(flags: &Option<String>) -> Value {
    match flags {
        None => Value::Null,
        Some(json) => serde_json::from_str(json).unwrap_or(Value::Null),
    }
}

/// Seeds a store from a case's `store` object.
fn seed(dir: &Path, spec: &Path, store: &Value) -> Bodies {
    let owner = PimdirStore::open_with_hash(dir, Some(PimdirHashAlgo::Blake3)).unwrap();
    let blobs = owner.blobs();
    let mut hashes = BTreeMap::new();
    let mut bytes = BTreeMap::new();
    for (label, object) in store["objects"].as_object().unwrap() {
        let body = fs::read(spec.join("vectors").join(object["body"].as_str().unwrap())).unwrap();
        let hash = blobs.hash(&body);
        let mut writer = blobs.writer().unwrap();
        std::io::Write::write_all(&mut writer, &body).unwrap();
        writer.commit(&hash).unwrap();
        hashes.insert(label.clone(), hash.0.clone());
        bytes.insert(label.clone(), body);
    }
    let bodies = Bodies { hashes, bytes };
    drop(owner);

    let conn = Connection::open(dir.join("pimdir.db")).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    for collection in store["collections"].as_array().unwrap() {
        conn.execute(
            sql::SET_COLLECTION_KIND,
            named_params! {
                ":collection": collection["id"].as_str().unwrap(),
                ":account": collection["account"].as_str(),
                ":kind": collection["kind"].as_str().unwrap_or(""),
            },
        )
        .unwrap();
        conn.execute(
            "UPDATE collections SET conflict = ?1, generation = ?2 WHERE id = ?3",
            params![
                collection["conflict"].as_str().unwrap_or("manual"),
                collection["generation"].as_i64().unwrap_or(1),
                collection["id"].as_str().unwrap(),
            ],
        )
        .unwrap();
    }
    for object in bodies.hashes.values() {
        let size = fs::metadata(blobs.path(&object.clone().into()))
            .unwrap()
            .len();
        conn.execute(
            sql::STORE_OBJECT,
            named_params! { ":hash": object, ":size": size as i64 },
        )
        .unwrap();
    }
    let mut next_seq = 1;
    for item in store["items"].as_array().unwrap() {
        let seq = item["seq"].as_i64().unwrap();
        next_seq = next_seq.max(seq + 1);
        conn.execute(
            sql::INSERT_ITEM,
            named_params! {
                ":collection": item["collection"].as_str().unwrap(),
                ":link_id": item["link_id"].as_str().unwrap(),
                ":seq": seq,
                ":flags": item.get("flags").map(|f| f.to_string()),
                ":object_hash": bodies.hash(&item["object"]),
                ":sort_key": item["sort_key"].as_str().unwrap_or(""),
                ":level": item["level"].as_i64().unwrap_or(0),
                ":deleted": item["deleted"].as_i64().unwrap_or(0),
                ":conflicted": item["conflicted"].as_i64().unwrap_or(0),
                ":conflict_object": bodies.hash(&item["conflict_object"]),
            },
        )
        .unwrap();
        if item["retained"].as_bool() == Some(true) {
            conn.execute(
                sql::RETAIN_ITEM,
                named_params! {
                    ":collection": item["collection"].as_str().unwrap(),
                    ":link_id": item["link_id"].as_str().unwrap(),
                    ":source": item["retained_by"].as_str(),
                },
            )
            .unwrap();
        }
    }
    conn.execute("UPDATE store_meta SET next_seq = ?1", params![next_seq])
        .unwrap();
    for binding in store["bindings"].as_array().unwrap() {
        // NOTE: a case states no agreement point (STORAGE §10), and a
        // binding whose source folded the item's body agrees with it: the
        // store would have moved shared_object with the absorbed upsert.
        let shared_object = match binding.get("shared_object") {
            Some(stated) => bodies.hash(stated),
            None => store["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| {
                    item["collection"] == binding["collection"]
                        && item["link_id"] == binding["link_id"]
                })
                .and_then(|item| bodies.hash(&item["object"])),
        };
        conn.execute(
            sql::INSERT_BINDING,
            named_params! {
                ":collection": binding["collection"].as_str().unwrap(),
                ":link_id": binding["link_id"].as_str().unwrap(),
                ":source": binding["source"].as_str().unwrap(),
                ":handle": binding["handle"].as_str().unwrap(),
                ":base_flags": binding.get("base_flags").filter(|f| !f.is_null()).map(|f| f.to_string()),
                ":base_object": bodies.hash(&binding["base_object"]),
                ":base_revision": binding["base_revision"].as_str(),
                ":base_present": binding["base_present"].as_i64().unwrap_or(0),
                ":conflicted": binding["conflicted"].as_i64().unwrap_or(0),
                ":conflict_revision": binding["conflict_revision"].as_str(),
                ":conflict_object": bodies.hash(&binding["conflict_object"]),
                ":shared_object": shared_object,
            },
        )
        .unwrap();
    }
    for probe in store["probes"].as_array().unwrap() {
        conn.execute(
            sql::UPSERT_PROBE,
            named_params! {
                ":collection": probe["collection"].as_str().unwrap(),
                ":source": probe["source"].as_str().unwrap(),
                ":handle": probe["handle"].as_str().unwrap(),
                ":flags": probe.get("flags").filter(|f| !f.is_null()).map(|f| f.to_string()),
            },
        )
        .unwrap();
    }
    for source in store["sources"].as_array().unwrap() {
        conn.execute(
            sql::UPSERT_CHECKPOINT,
            named_params! {
                ":collection": source["collection"].as_str().unwrap(),
                ":source": source["source"].as_str().unwrap(),
                ":checkpoint": source["checkpoint"].as_str().map(str::as_bytes),
            },
        )
        .unwrap();
    }
    conn.execute(sql::RECOMPUTE_REFCOUNTS, []).unwrap();

    bodies
}

/// The remote a case scripts: one snapshot, fetch answers by handle and
/// tier, outcomes by handle, and every push it was handed, recorded on
/// the terms SYNC §11 compares a push on.
struct Scripted {
    spec: PathBuf,
    kind: String,
    algo: PimdirHashAlgo,
    bodies: Bodies,
    snapshot: Option<PimdirRemoteSnapshot>,
    fetch: Map<String, Value>,
    outcomes: Vec<Value>,
    pushes: Vec<Value>,
}

impl Scripted {
    /// One push as the case's `expect.pushes` spells it.
    fn record(&self, change: &PimdirChange) -> Value {
        let handle = change.handle().0.clone();
        let key = change.key.as_str();
        let label =
            |hash: &Option<PimdirHash>| self.bodies.label(hash.as_ref().map(|h| h.0.clone()));
        match &change.kind {
            PimdirChangeKind::SetFlags { flags, .. } => {
                json!({ "kind": "SetFlags", "handle": handle, "key": key, "flags": flags.known() })
            }
            PimdirChangeKind::Remove {
                to,
                link_id,
                if_match,
                ..
            } => json!({
                "kind": "Remove",
                "handle": handle,
                "key": key,
                "to": to.as_ref().map(|t| t.0.clone()),
                "link_id": link_id.as_ref().map(|l| l.0.clone()),
                "if_match": if_match,
            }),
            PimdirChangeKind::Update {
                object, if_match, ..
            } => json!({
                "kind": "Update",
                "handle": handle,
                "key": key,
                "object": label(&Some(object.clone())),
                "if_match": if_match,
            }),
            PimdirChangeKind::Add {
                link_id,
                flags,
                origin,
                object,
                ..
            } => json!({
                "kind": "Add",
                "handle": handle,
                "key": key,
                "link_id": link_id.as_ref().map(|l| l.0.clone()),
                "flags": flags.known(),
                "origin": origin.as_ref().map(|o| json!({
                    "collection": o.collection.0,
                    "handle": o.handle.0,
                })),
                "object": label(object),
            }),
        }
    }
}

impl PimdirRemote for Scripted {
    type Error = String;

    fn enumerate(
        &mut self,
        _: &PimdirCollectionId,
        _: Option<PimdirCheckpoint>,
    ) -> Result<PimdirRemoteSnapshot, String> {
        self.snapshot
            .take()
            .ok_or_else(|| "no snapshot scripted".into())
    }

    fn fetch(
        &mut self,
        _: &PimdirCollectionId,
        handles: Vec<PimdirHandle>,
        tier: PimdirTier,
    ) -> Result<Vec<PimdirFetchedItem>, String> {
        let tier_name = match tier {
            PimdirTier::Meta => "Meta",
            PimdirTier::Full => "Full",
        };
        let mut items = Vec::new();
        for handle in handles {
            let Some(answer) = self.fetch.get(&handle.0).and_then(|t| t.get(tier_name)) else {
                continue;
            };
            let body = fs::read(
                self.spec
                    .join("vectors")
                    .join(answer["body"].as_str().unwrap()),
            )
            .unwrap();
            let derivation = summary::derive(&self.kind, &body).expect("a known kind");
            let mut summary = derivation.summary;
            // NOTE: a Meta fetch answers from an ENVELOPE, which walks no
            // part; the fixture stands in for one (Annex A.1).
            if let (PimdirTier::Meta, Some(PimdirSummary::Mail(mail))) = (tier, &mut summary) {
                mail.attachment = None;
                mail.size = Some(body.len() as u64);
            }
            items.push(PimdirFetchedItem {
                handle,
                link_id: derivation.link_id,
                summary,
                sort_key: derivation.sort_key,
                body: match tier {
                    PimdirTier::Meta => None,
                    PimdirTier::Full => Some(PimdirFetchedBody::Inline {
                        hash: self.algo.hash(&body),
                        bytes: body,
                    }),
                },
                revision: answer["revision"].as_str().map(String::from),
            });
        }
        Ok(items)
    }

    fn push(
        &mut self,
        _: &PimdirCollectionId,
        changes: Vec<PimdirChange>,
    ) -> Result<Vec<PimdirPushResult>, String> {
        let mut results = Vec::new();
        for change in changes {
            self.pushes.push(self.record(&change));
            let handle = change.handle().clone();
            let scripted = self
                .outcomes
                .iter()
                .find(|o| o["handle"].as_str() == Some(&handle.0));
            results.push(PimdirPushResult {
                handle,
                outcome: match scripted.and_then(|o| o["outcome"].as_str()) {
                    Some("Rejected") => PimdirPushOutcome::Rejected,
                    _ => PimdirPushOutcome::Accepted,
                },
                assigned: scripted
                    .and_then(|o| o["assigned"].as_str())
                    .map(PimdirHandle::from),
                revision: scripted
                    .and_then(|o| o["revision"].as_str())
                    .map(String::from),
            });
        }
        Ok(results)
    }
}

fn options(value: &Value) -> PimdirSyncOptions {
    let rights = &value["rights"];
    PimdirSyncOptions {
        push: value["push"].as_bool().unwrap_or(true),
        rights: PimdirPushRights {
            flags: rights["flags"].as_bool().unwrap_or(true),
            content: rights["content"].as_bool().unwrap_or(true),
            add: rights["add"].as_bool().unwrap_or(true),
            remove: rights["remove"].as_bool().unwrap_or(true),
        },
        delete: match value["delete"].as_str() {
            Some("keep") => PimdirDeletePolicy::Keep,
            Some("revert") => PimdirDeletePolicy::Revert,
            _ => PimdirDeletePolicy::Auto,
        },
        conflict: match value["conflict"].as_str() {
            Some("prefer-local") => PimdirConflictPolicy::PreferLocal,
            Some("prefer-remote") => PimdirConflictPolicy::PreferRemote,
            Some("keep-both") => PimdirConflictPolicy::KeepBoth,
            _ => PimdirConflictPolicy::Manual,
        },
        full: false,
    }
}

/// The mutation a `mutate` run carries (SYNC §7), bodies by label.
fn mutation(value: &Value, kind: &str, bodies: &Bodies) -> PimdirMutation {
    let handle = || PimdirHandle::from(value["handle"].as_str().unwrap());
    let target = || PimdirCollectionId::from(value["target"].as_str().unwrap());
    let placeholder = || PimdirHandle::from(value["placeholder"].as_str().unwrap());
    let derived = |body: &[u8]| summary::derive(kind, body);

    match value["kind"].as_str().unwrap() {
        "SetFlags" => PimdirMutation::SetFlags {
            handle: handle(),
            flags: flags(&value["flags"]),
        },
        "Remove" => PimdirMutation::Remove(handle()),
        "Edit" => {
            let (object, body) = bodies.object(&value["object"]);
            let derivation = derived(&body);
            PimdirMutation::Edit {
                handle: handle(),
                object,
                summary: derivation.as_ref().and_then(|d| d.summary.clone()),
                sort_key: derivation.map(|d| d.sort_key),
                body,
            }
        }
        "Copy" => PimdirMutation::Copy {
            handle: handle(),
            target: target(),
            placeholder: placeholder(),
        },
        "Move" => PimdirMutation::Move {
            handle: handle(),
            target: target(),
            placeholder: placeholder(),
        },
        "Add" => {
            let (object, body) = bodies.object(&value["object"]);
            let derivation = derived(&body);
            let link_id = value["link_id"]
                .as_str()
                .map(PimdirLinkId::from)
                .or_else(|| derivation.as_ref().map(|d| d.link_id.clone()))
                .expect("a link id, stated or derived");
            PimdirMutation::Add {
                handle: handle(),
                link_id,
                flags: flags(&value["flags"]),
                object,
                summary: derivation.as_ref().and_then(|d| d.summary.clone()),
                sort_key: derivation.map(|d| d.sort_key).unwrap_or_default(),
                body,
            }
        }
        kind => panic!("unsupported mutation {kind}"),
    }
}

/// Runs one case's verb, returning the pushes and the events.
fn run(
    store: &mut PimdirSourceStore,
    case: &Value,
    remote: &mut Scripted,
) -> (Vec<Value>, Vec<Value>) {
    let run = &case["run"];
    let collection = run["collection"].as_str().unwrap();
    let events = match run["verb"].as_str().unwrap() {
        "sync" => store
            .sync(collection, options(&run["options"]), remote)
            .unwrap()
            .events
            .iter()
            .map(|event| {
                let (kind, handle) = match event {
                    PimdirSyncEvent::Added(h) => ("Added", h),
                    PimdirSyncEvent::FlagsChanged(h) => ("FlagsChanged", h),
                    PimdirSyncEvent::ContentChanged(h) => ("ContentChanged", h),
                    PimdirSyncEvent::Vanished(h) => ("Vanished", h),
                    PimdirSyncEvent::Conflicted(h) => ("Conflicted", h),
                    PimdirSyncEvent::Created(h) => ("Created", h),
                };
                json!({ "kind": kind, "handle": handle.0 })
            })
            .collect(),
        "upgrade" => {
            let handles = run["handles"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .map(PimdirHandle::from)
                .collect();
            let tier = match run["tier"].as_str() {
                Some("Full") => PimdirTier::Full,
                _ => PimdirTier::Meta,
            };
            store.upgrade(collection, handles, tier, remote).unwrap();
            Vec::new()
        }
        "mutate" => {
            let mutation = mutation(&run["mutation"], &remote.kind, &remote.bodies);
            store.mutate(collection, mutation).unwrap();
            Vec::new()
        }
        "rekey" => {
            store.rekey(collection, remote).unwrap();
            Vec::new()
        }
        "open" => {
            store.open_collection(collection).unwrap();
            Vec::new()
        }
        verb => panic!("unsupported verb {verb}"),
    };

    (std::mem::take(&mut remote.pushes), events)
}

/// Every row of a table the case expects, projected onto the keys the
/// expected rows carry, bodies by label.
fn actual_rows(conn: &Connection, bodies: &Bodies, table: &str, expected: &[Value]) -> Vec<Value> {
    let keys: Vec<String> = expected
        .first()
        .and_then(Value::as_object)
        .map(|row| row.keys().cloned().collect())
        .unwrap_or_default();

    let query = match table {
        "items" => {
            "SELECT collection, link_id, seq, flags, object_hash, level, deleted, retained_at, retained_by, sort_key, conflicted, conflict_object FROM items ORDER BY collection, link_id"
        }
        "bindings" => {
            "SELECT collection, link_id, source, handle, base_flags, base_object, base_revision, base_present, conflicted, conflict_revision, conflict_object, shared_object FROM bindings ORDER BY collection, link_id, source"
        }
        "probes" => {
            "SELECT collection, source, handle, flags FROM probes ORDER BY collection, source, handle"
        }
        "sources" => {
            "SELECT collection, source, checkpoint FROM sources ORDER BY collection, source"
        }
        "collections" => {
            "SELECT id, account, kind, conflict, generation FROM collections ORDER BY id"
        }
        "addresses" => {
            "SELECT collection, link_id, role, position, address, name FROM item_address ORDER BY collection, link_id, role, position"
        }
        _ => panic!("unknown table {table}"),
    };

    let mut stmt = conn.prepare(query).unwrap();
    let rows = stmt
        .query_map([], |row| {
            let mut object = Map::new();
            let mut get = |name: &str, value: Value| {
                if keys.iter().any(|key| key == name) {
                    object.insert(name.to_string(), value);
                }
            };
            match table {
                "items" => {
                    get("collection", json!(row.get::<_, String>(0)?));
                    get("link_id", json!(row.get::<_, String>(1)?));
                    get("seq", json!(row.get::<_, i64>(2)?));
                    get("flags", flags_json(&row.get::<_, Option<String>>(3)?));
                    get("object", bodies.label(row.get(4)?));
                    get("level", json!(row.get::<_, i64>(5)?));
                    get("deleted", json!(row.get::<_, i64>(6)?));
                    get(
                        "retained",
                        json!(row.get::<_, Option<String>>(7)?.is_some()),
                    );
                    get("retained_by", json!(row.get::<_, Option<String>>(8)?));
                    get("sort_key", json!(row.get::<_, String>(9)?));
                    get("conflicted", json!(row.get::<_, i64>(10)?));
                    get("conflict_object", bodies.label(row.get(11)?));
                }
                "bindings" => {
                    get("collection", json!(row.get::<_, String>(0)?));
                    get("link_id", json!(row.get::<_, String>(1)?));
                    get("source", json!(row.get::<_, String>(2)?));
                    get("handle", json!(row.get::<_, String>(3)?));
                    get("base_flags", flags_json(&row.get::<_, Option<String>>(4)?));
                    get("base_object", bodies.label(row.get(5)?));
                    get("base_revision", json!(row.get::<_, Option<String>>(6)?));
                    get("base_present", json!(row.get::<_, i64>(7)?));
                    get("conflicted", json!(row.get::<_, i64>(8)?));
                    get("conflict_revision", json!(row.get::<_, Option<String>>(9)?));
                    get("conflict_object", bodies.label(row.get(10)?));
                    get("shared_object", bodies.label(row.get(11)?));
                }
                "probes" => {
                    get("collection", json!(row.get::<_, String>(0)?));
                    get("source", json!(row.get::<_, String>(1)?));
                    get("handle", json!(row.get::<_, String>(2)?));
                    get("flags", flags_json(&row.get::<_, Option<String>>(3)?));
                }
                "sources" => {
                    get("collection", json!(row.get::<_, String>(0)?));
                    get("source", json!(row.get::<_, String>(1)?));
                    let checkpoint: Option<Vec<u8>> = row.get(2)?;
                    get(
                        "checkpoint",
                        json!(checkpoint.map(|bytes| String::from_utf8_lossy(&bytes).into_owned())),
                    );
                }
                "collections" => {
                    get("id", json!(row.get::<_, String>(0)?));
                    get("account", json!(row.get::<_, Option<String>>(1)?));
                    get("kind", json!(row.get::<_, String>(2)?));
                    get("conflict", json!(row.get::<_, String>(3)?));
                    get("generation", json!(row.get::<_, i64>(4)?));
                }
                "addresses" => {
                    get("collection", json!(row.get::<_, String>(0)?));
                    get("link_id", json!(row.get::<_, String>(1)?));
                    get("role", json!(row.get::<_, String>(2)?));
                    get("position", json!(row.get::<_, i64>(3)?));
                    get("address", json!(row.get::<_, String>(4)?));
                    get("name", json!(row.get::<_, Option<String>>(5)?));
                }
                _ => unreachable!(),
            }
            Ok(Value::Object(object))
        })
        .unwrap();

    rows.map(Result::unwrap).collect()
}

/// The summary rows the case expects, read from their tables.
fn actual_summaries(conn: &Connection, expected: &[Value]) -> Vec<Value> {
    expected
        .iter()
        .map(|entry| {
            let table = entry["table"].as_str().unwrap();
            let columns: Vec<String> = entry["row"].as_object().unwrap().keys().cloned().collect();
            let query = format!(
                "SELECT {} FROM {table} WHERE collection = ?1 AND link_id = ?2",
                columns.join(", ")
            );
            let row = conn
                .query_row(
                    &query,
                    params![
                        entry["collection"].as_str().unwrap(),
                        entry["link_id"].as_str().unwrap()
                    ],
                    |row| {
                        let mut object = Map::new();
                        for (at, column) in columns.iter().enumerate() {
                            let value: rusqlite::types::Value = row.get(at)?;
                            let value = match value {
                                rusqlite::types::Value::Null => Value::Null,
                                rusqlite::types::Value::Integer(i) => json!(i),
                                rusqlite::types::Value::Real(f) => json!(f),
                                rusqlite::types::Value::Text(t) if column == "in_reply_to" => {
                                    serde_json::from_str(&t).unwrap_or(Value::Null)
                                }
                                rusqlite::types::Value::Text(t) => json!(t),
                                rusqlite::types::Value::Blob(_) => Value::Null,
                            };
                            object.insert(column.clone(), value);
                        }
                        Ok(Value::Object(object))
                    },
                )
                .unwrap_or(Value::Null);
            json!({
                "table": table,
                "collection": entry["collection"],
                "link_id": entry["link_id"],
                "row": row,
            })
        })
        .collect()
}

fn assert_same_set(label: &str, table: &str, mut expected: Vec<Value>, mut actual: Vec<Value>) {
    let key = |value: &Value| value.to_string();
    expected.sort_by_key(key);
    actual.sort_by_key(key);
    assert_eq!(
        expected, actual,
        "{label}: {table} rows differ\nexpected {expected:#?}\nactual {actual:#?}"
    );
}

/// The pushes in order, each projected onto the keys the case carries for
/// it (SYNC §11): kind, handle, key, and what the kind carries.
fn assert_same_pushes(label: &str, expected: &[Value], actual: &[Value]) {
    assert_eq!(
        expected.len(),
        actual.len(),
        "{label}: pushes differ\nexpected {expected:#?}\nactual {actual:#?}"
    );
    for (expected, actual) in expected.iter().zip(actual) {
        let projected: Map<String, Value> = expected
            .as_object()
            .unwrap()
            .keys()
            .map(|key| (key.clone(), actual[key].clone()))
            .collect();
        assert_eq!(
            expected,
            &Value::Object(projected),
            "{label}: push differs\nexpected {expected:#?}\nactual {actual:#?}"
        );
    }
}

#[test]
fn every_sync_vector_reproduces() {
    let Some(spec) = spec_dir() else {
        println!("skipped: no pimdir spec checkout beside this one");
        return;
    };

    let mut paths: Vec<PathBuf> = fs::read_dir(spec.join("vectors/sync"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "the sync vectors carry no case");

    for path in paths {
        let case: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let label = format!(
            "{}: {}",
            path.file_name().unwrap().to_string_lossy(),
            case["label"].as_str().unwrap_or("")
        );
        println!("{label}");
        let dir = tempfile::tempdir().unwrap();
        let bodies = seed(dir.path(), &spec, &case["store"]);

        let run_spec = &case["run"];
        let collection = run_spec["collection"].as_str().unwrap();
        let kind = case["store"]["collections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"].as_str() == Some(collection))
            .and_then(|c| c["kind"].as_str())
            .unwrap_or("")
            .to_string();
        let snapshot =
            case["remote"]["snapshot"]
                .as_object()
                .map(|snapshot| PimdirRemoteSnapshot {
                    items: snapshot["items"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|item| PimdirRemoteItem {
                            handle: PimdirHandle::from(item["handle"].as_str().unwrap()),
                            flags: flags(&item["flags"]),
                            revision: item["revision"].as_str().map(String::from),
                        })
                        .collect(),
                    vanished: snapshot["vanished"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(Value::as_str)
                        .map(PimdirHandle::from)
                        .collect(),
                    complete: snapshot["complete"].as_bool().unwrap_or(true),
                    checkpoint: PimdirCheckpoint(
                        snapshot["checkpoint"].as_str().unwrap().as_bytes().to_vec(),
                    ),
                });
        let mut remote = Scripted {
            spec: spec.clone(),
            kind,
            algo: PimdirHashAlgo::Blake3,
            bodies: bodies.clone(),
            snapshot,
            fetch: case["remote"]["fetch"]
                .as_object()
                .cloned()
                .unwrap_or_default(),
            outcomes: case["expect"]["outcomes"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
            pushes: Vec::new(),
        };

        let mut store = PimdirStore::open(dir.path())
            .unwrap()
            .for_source(run_spec["source"].as_str().unwrap());
        let (pushes, events) = run(&mut store, &case, &mut remote);
        drop(store);

        let expected_pushes = case["expect"]["pushes"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert_same_pushes(&label, &expected_pushes, &pushes);
        assert_eq!(
            case["expect"]["events"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
            events,
            "{label}: events differ"
        );

        let conn = Connection::open(dir.path().join("pimdir.db")).unwrap();
        let expect = case["expect"]["store"].as_object().unwrap();
        for (table, rows) in expect {
            let expected = rows.as_array().cloned().unwrap_or_default();
            let actual = match table.as_str() {
                "summaries" => actual_summaries(&conn, &expected),
                table => actual_rows(&conn, &bodies, table, &expected),
            };
            assert_same_set(&label, table, expected, actual);
        }
    }
}
