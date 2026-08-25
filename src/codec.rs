//! Pure, I/O-free encodings between the [`io_replica`] model and the pimdir
//! columns (spec §13). No SQLite, no filesystem: this is the part an Android or
//! any other implementation reuses.
//!
//! Beside the column encodings, this module holds the action-queue payload
//! codec (spec §15.3): the six v1 action kinds as [`PimdirAction`], encoded to
//! and from the versioned JSON `queue.payload` column.

use alloc::{
    string::{String, ToString},
    vec::Vec,
};
use core::fmt;

use io_replica::{
    collection::ReplicaCollectionId,
    object::ReplicaHash,
    placement::{ReplicaFlags, ReplicaHandle, ReplicaLevel, ReplicaLinkId, ReplicaMeta},
};
use serde_json::{Map, Value, json};

/// A flag set to its canonical JSON array (sorted; the model's set is already
/// ordered), or `None` for the column's `NULL`.
///
/// The two absences the spec (§13) keeps apart: a known-empty set encodes as
/// `"[]"`, and a set nobody has read encodes as `NULL`. Collapsing them would
/// have a probed item claim to carry no markers.
pub fn flags_to_json(flags: &ReplicaFlags) -> Option<String> {
    let items: Vec<&String> = flags.known()?.iter().collect();
    Some(serde_json::to_string(&items).unwrap_or_else(|_| String::from("[]")))
}

/// The inverse of [`flags_to_json`]: a `NULL` column (passed as `None`) decodes
/// to the unknown set, and so does a column this cannot read.
///
/// Malformed JSON is a column written by something whose format this does not
/// share, or one that was corrupted, and neither is evidence about the item's
/// markers. Reading it as a known-empty set turns that into an authoritative
/// "this item carries no markers", which the merge takes as one side's opinion:
/// it clears every marker the other side reports and persists the result, so a
/// read failure becomes permanent loss. Unknown holds no opinion instead.
pub fn flags_from_json(json: Option<&str>) -> ReplicaFlags {
    let Some(json) = json else {
        return ReplicaFlags::Unknown;
    };
    match serde_json::from_str::<Vec<String>>(json) {
        Ok(items) => ReplicaFlags::Known(items.into_iter().collect()),
        Err(_) => ReplicaFlags::Unknown,
    }
}

/// The detail ladder as its column integer (spec §13).
pub fn level_to_int(level: ReplicaLevel) -> i64 {
    match level {
        ReplicaLevel::Probed => 0,
        ReplicaLevel::Meta => 1,
        ReplicaLevel::Full => 2,
    }
}

/// The inverse of [`level_to_int`]; unknown integers clamp to `Probed`.
pub fn level_from_int(value: i64) -> ReplicaLevel {
    match value {
        1 => ReplicaLevel::Meta,
        2 => ReplicaLevel::Full,
        _ => ReplicaLevel::Probed,
    }
}

/// A queued mutation request (spec §15.3): what a producer appends to the
/// `queue` table and the owner applies to the store.
///
/// The kinds mirror io-replica's mutation vocabulary on purpose: the queue is
/// the cross-process projection of the engine's mutate verb. Existing items are
/// addressed by their public `seq` (spec §9.1), the same identifier a reading
/// client already holds; the owner resolves it back to the internal link id.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirAction {
    /// Create an item in the collection, staged as a local creation for the
    /// sync layer to push. A duplicate live `link_id` in the collection parks
    /// the action (the item already exists).
    Add {
        /// The item's cross-source link id; `None` derives it from `object`.
        link_id: Option<ReplicaLinkId>,
        /// The initial flag set.
        flags: ReplicaFlags,
        /// The body's content hash, matching the row's `object_hash`; the
        /// producer wrote the blob durably before enqueueing.
        object: Option<ReplicaHash>,
        /// The item's summary (spec Annex A), or `None` when not projected.
        meta: Option<ReplicaMeta>,
        /// The provisional handle the create is staged under; `None` lets the
        /// owner derive one.
        handle: Option<ReplicaHandle>,
    },
    /// Replace the item's flag set (absolute, never a delta, so reapplication
    /// is idempotent).
    SetFlags {
        /// The item's public id.
        seq: i64,
        /// The new flag set.
        flags: ReplicaFlags,
    },
    /// Remove the item from the collection; already-absent is success, not an
    /// error.
    Remove {
        /// The item's public id.
        seq: i64,
    },
    /// Refile the item into another collection (target create plus source
    /// removal).
    Move {
        /// The item's public id.
        seq: i64,
        /// The collection to move it into.
        to: ReplicaCollectionId,
    },
    /// Copy the item into another collection (same as a move, without the
    /// removal).
    Copy {
        /// The item's public id.
        seq: i64,
        /// The collection to copy it into.
        to: ReplicaCollectionId,
    },
    /// Repoint a mutable-content item's body (a contact or event edit).
    Update {
        /// The item's public id.
        seq: i64,
        /// The new body's content hash; the producer wrote the blob durably
        /// before enqueueing.
        object: ReplicaHash,
        /// The refreshed summary, or `None` to keep the cached one.
        meta: Option<ReplicaMeta>,
    },
    /// An action this crate defines no semantics for: an owner-defined intent
    /// (a mail submission) carried by the same queue as the store mutations
    /// above.
    ///
    /// The store cannot apply it, so its drain **skips** the row rather than
    /// parking it: the action is not unappliable, only unappliable *here*. An
    /// owner that recognises the kind inspects it, performs it out of band and
    /// acknowledges it with [`drop_action`]. Only a genuinely malformed payload
    /// (not JSON, no supported `v`) parks.
    ///
    /// [`drop_action`]: ../client/struct.PimdirStore.html#method.drop_action
    Unknown {
        /// The raw `queue.action` kind.
        kind: String,
        /// The raw versioned JSON payload, verbatim: only its owner knows the
        /// shape, so nothing here re-encodes it.
        payload: String,
        /// The body hash the payload's `object` field names, by the same
        /// convention as the known kinds, so an intent carrying a body pins it
        /// against garbage collection like any other queued body.
        object_hash: Option<ReplicaHash>,
    },
}

impl PimdirAction {
    /// The action kind as its `queue.action` column value (spec §13).
    pub fn kind(&self) -> &str {
        match self {
            Self::Add { .. } => "add",
            Self::SetFlags { .. } => "set-flags",
            Self::Remove { .. } => "remove",
            Self::Move { .. } => "move",
            Self::Copy { .. } => "copy",
            Self::Update { .. } => "update",
            Self::Unknown { kind, .. } => kind,
        }
    }

    /// The body hash the payload references, if any: the value the enqueue
    /// carries in `queue.object_hash` so the pending body is pinned against
    /// garbage collection (spec §15.1).
    pub fn object_hash(&self) -> Option<&ReplicaHash> {
        match self {
            Self::Add { object, .. } => object.as_ref(),
            Self::Update { object, .. } => Some(object),
            Self::Unknown { object_hash, .. } => object_hash.as_ref(),
            Self::SetFlags { .. } | Self::Remove { .. } | Self::Move { .. } | Self::Copy { .. } => {
                None
            }
        }
    }
}

/// A malformed action payload; the owner parks the row instead of applying it.
///
/// An unrecognised *kind* is not one of these: it decodes as
/// [`PimdirAction::Unknown`] and is skipped, since another owner may be able to
/// perform it. Only a payload no owner could act on lands here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirActionError {
    /// The payload is not a JSON object.
    Json,
    /// The payload's leading `v` is missing or not a supported version.
    UnknownVersion(Option<i64>),
    /// A required payload field is missing or has the wrong shape.
    MissingField(&'static str),
}

impl fmt::Display for PimdirActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json => write!(f, "pimdir action payload is not a JSON object"),
            Self::UnknownVersion(Some(v)) => write!(f, "unknown pimdir action version: {v}"),
            Self::UnknownVersion(None) => write!(f, "pimdir action payload misses its version"),
            Self::MissingField(field) => write!(f, "pimdir action payload misses field: {field}"),
        }
    }
}

impl core::error::Error for PimdirActionError {}

/// Encodes an action to its versioned JSON `queue.payload` column (spec §15.3,
/// `v: 1`). Absent optional fields are omitted; `meta` embeds as parsed JSON
/// when it is valid JSON, else as a JSON string.
pub fn action_to_payload(action: &PimdirAction) -> String {
    // NOTE: an owner-defined intent round-trips byte for byte; this crate knows
    // no more of its shape than the `object` field it pins.
    if let PimdirAction::Unknown { payload, .. } = action {
        return payload.clone();
    }

    let mut map = Map::new();
    map.insert("v".into(), json!(1));

    match action {
        PimdirAction::Add {
            link_id,
            flags,
            object,
            meta,
            handle,
        } => {
            if let Some(link) = link_id {
                map.insert("link_id".into(), json!(link.0));
            }
            map.insert("flags".into(), flags_to_value(flags));
            if let Some(object) = object {
                map.insert("object".into(), json!(object.0));
            }
            if let Some(meta) = meta {
                map.insert("meta".into(), meta_to_value(meta));
            }
            if let Some(handle) = handle {
                map.insert("handle".into(), json!(handle.0));
            }
        }
        PimdirAction::SetFlags { seq, flags } => {
            map.insert("seq".into(), json!(seq));
            map.insert("flags".into(), flags_to_value(flags));
        }
        PimdirAction::Remove { seq } => {
            map.insert("seq".into(), json!(seq));
        }
        PimdirAction::Move { seq, to } | PimdirAction::Copy { seq, to } => {
            map.insert("seq".into(), json!(seq));
            map.insert("to".into(), json!(to.0));
        }
        PimdirAction::Update { seq, object, meta } => {
            map.insert("seq".into(), json!(seq));
            map.insert("object".into(), json!(object.0));
            if let Some(meta) = meta {
                map.insert("meta".into(), meta_to_value(meta));
            }
        }
        // NOTE: returned verbatim above.
        PimdirAction::Unknown { .. } => {}
    }

    Value::Object(map).to_string()
}

/// Decodes a `queue.action` kind plus its `queue.payload` JSON back to a
/// [`PimdirAction`] — the inverse of [`action_to_payload`]. Strict, unlike the
/// lenient column decoders: a malformed payload is an error the owner parks
/// the row with, never a silently-empty action.
pub fn action_from_payload(kind: &str, payload: &str) -> Result<PimdirAction, PimdirActionError> {
    let value: Value = serde_json::from_str(payload).map_err(|_| PimdirActionError::Json)?;
    let map = value.as_object().ok_or(PimdirActionError::Json)?;

    let version = map.get("v").and_then(Value::as_i64);
    if version != Some(1) {
        return Err(PimdirActionError::UnknownVersion(version));
    }

    match kind {
        "add" => Ok(PimdirAction::Add {
            link_id: get_string(map, "link_id")?.map(ReplicaLinkId),
            flags: flags_from_value(map.get("flags")),
            object: get_string(map, "object")?.map(ReplicaHash),
            meta: map.get("meta").map(meta_from_value),
            handle: get_string(map, "handle")?.map(ReplicaHandle),
        }),
        "set-flags" => Ok(PimdirAction::SetFlags {
            seq: require_seq(map)?,
            flags: flags_from_value(map.get("flags")),
        }),
        "remove" => Ok(PimdirAction::Remove {
            seq: require_seq(map)?,
        }),
        "move" => Ok(PimdirAction::Move {
            seq: require_seq(map)?,
            to: ReplicaCollectionId(require_string(map, "to")?),
        }),
        "copy" => Ok(PimdirAction::Copy {
            seq: require_seq(map)?,
            to: ReplicaCollectionId(require_string(map, "to")?),
        }),
        "update" => Ok(PimdirAction::Update {
            seq: require_seq(map)?,
            object: ReplicaHash(require_string(map, "object")?),
            meta: map.get("meta").map(meta_from_value),
        }),
        // NOTE: an owner-defined intent, not a malformed row: the payload is
        // well-formed and versioned, this crate simply defines no semantics for
        // the kind. Kept whole for the owner that does, which is what lets one
        // queue carry store mutations beside capability-bound intents.
        other => Ok(PimdirAction::Unknown {
            kind: other.to_string(),
            payload: payload.to_string(),
            object_hash: get_string(map, "object")?.map(ReplicaHash),
        }),
    }
}

/// A flag set as a JSON array value (the model's set is already sorted), or
/// `null` for an unknown one.
///
/// An action states an intent, so its set is known in every payload the spec
/// defines (§15.3). Encoding an unknown one as `null` rather than as `[]` keeps
/// a nonsensical action legible instead of turning it into a deliberate
/// clearing of every flag.
fn flags_to_value(flags: &ReplicaFlags) -> Value {
    match flags.known() {
        None => Value::Null,
        Some(flags) => Value::Array(flags.iter().map(|f| json!(f)).collect()),
    }
}

/// The inverse of [`flags_to_value`]; an absent or `null` value decodes to the
/// unknown set and a malformed array to a known-empty one, matching
/// [`flags_from_json`].
fn flags_from_value(value: Option<&Value>) -> ReplicaFlags {
    match value {
        None | Some(Value::Null) => ReplicaFlags::Unknown,
        Some(value) => ReplicaFlags::Known(
            value
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default(),
        ),
    }
}

/// A stored summary embedded in a payload: parsed JSON when the opaque meta is
/// valid JSON (the spec Annex A conventions), a JSON string otherwise.
fn meta_to_value(meta: &ReplicaMeta) -> Value {
    serde_json::from_str(&meta.0).unwrap_or_else(|_| json!(meta.0))
}

/// The inverse of [`meta_to_value`]: a JSON string is taken verbatim, any other
/// value is re-serialised to the stored opaque form.
fn meta_from_value(value: &Value) -> ReplicaMeta {
    match value.as_str() {
        Some(text) => ReplicaMeta(text.to_string()),
        None => ReplicaMeta(value.to_string()),
    }
}

/// An optional string payload field; present with a non-string shape errors.
fn get_string(
    map: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, PimdirActionError> {
    match map.get(field) {
        None => Ok(None),
        Some(value) => match value.as_str() {
            Some(text) => Ok(Some(text.to_string())),
            None => Err(PimdirActionError::MissingField(field)),
        },
    }
}

/// A required string payload field.
fn require_string(
    map: &Map<String, Value>,
    field: &'static str,
) -> Result<String, PimdirActionError> {
    get_string(map, field)?.ok_or(PimdirActionError::MissingField(field))
}

/// The required `seq` payload field (an integer public id).
fn require_seq(map: &Map<String, Value>) -> Result<i64, PimdirActionError> {
    map.get("seq")
        .and_then(Value::as_i64)
        .ok_or(PimdirActionError::MissingField("seq"))
}

/// The other handles a source holds one identity under, as the JSON array the
/// column stores; `None` for the ordinary case of none.
///
/// Recorded rather than inferred: a source holds an identity twice for exactly
/// as long as it does, and the enumeration that reveals it says so once.
pub fn handles_to_json(handles: &[ReplicaHandle]) -> Option<String> {
    if handles.is_empty() {
        return None;
    }
    let items: Vec<&str> = handles.iter().map(|h| h.0.as_str()).collect();
    serde_json::to_string(&items).ok()
}

/// The inverse of [`handles_to_json`]. A column this cannot read decodes to
/// none, on the same terms as a flag set: it is not evidence of a duplicate,
/// and freezing an item on an unreadable column would strand it.
pub fn handles_from_json(json: Option<&str>) -> Vec<ReplicaHandle> {
    let Some(json) = json else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(json)
        .map(|items| items.into_iter().map(ReplicaHandle).collect())
        .unwrap_or_default()
}
#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn flags_round_trip_and_escape() {
        let flags = ReplicaFlags::from_iter(["\\Seen", "$flagged", "a\"b"]);
        let json = flags_to_json(&flags).expect("a known set encodes");
        assert_eq!(flags_from_json(Some(&json)), flags);
        // NOTE: canonical (sorted) and JSON-escaped.
        assert!(json.starts_with('['));
        assert!(json.contains("\\\\Seen"));
    }

    #[test]
    fn an_unknown_set_is_null_and_a_known_empty_one_is_a_list() {
        // The two absences the store keeps apart (spec §13): NULL means the
        // markers were never read, '[]' means the item carries none.
        assert_eq!(flags_to_json(&ReplicaFlags::Unknown), None);
        assert_eq!(flags_from_json(None), ReplicaFlags::Unknown);

        assert_eq!(
            flags_to_json(&ReplicaFlags::default()).as_deref(),
            Some("[]")
        );
        assert_eq!(flags_from_json(Some("[]")), ReplicaFlags::default());
    }

    #[test]
    fn a_malformed_flag_set_reads_as_unread_not_as_empty() {
        // A decode failure must not become an authoritative "this item
        // carries no markers": the merge would take that as one side's
        // opinion, clear every marker the other side reports, and persist
        // the result. Unknown holds no opinion, so the markers survive
        // wherever they are still readable.
        assert_eq!(flags_from_json(Some("not json")), ReplicaFlags::Unknown);
        assert_eq!(flags_from_json(Some("{}")), ReplicaFlags::Unknown);
    }

    #[test]
    fn ambiguous_handles_round_trip_and_none_is_null() {
        // The column distinguishes "no duplicate" from "a duplicate": NULL is
        // the ordinary case, and a list is the freeze.
        assert_eq!(handles_to_json(&[]), None);
        assert!(handles_from_json(None).is_empty());

        let handles = vec![ReplicaHandle("u2".into()), ReplicaHandle("u3".into())];
        let json = handles_to_json(&handles).expect("a non-empty list encodes");
        assert_eq!(handles_from_json(Some(&json)), handles);

        // A column this cannot read is not evidence of a duplicate, and
        // freezing an item on one would strand it.
        assert!(handles_from_json(Some("not json")).is_empty());
    }

    #[test]
    fn level_map_round_trips() {
        for l in [ReplicaLevel::Probed, ReplicaLevel::Meta, ReplicaLevel::Full] {
            assert_eq!(level_from_int(level_to_int(l)), l);
        }
    }

    #[test]
    fn every_action_kind_round_trips_through_its_payload() {
        let actions = [
            PimdirAction::Add {
                link_id: Some(ReplicaLinkId("mid:new".into())),
                flags: ReplicaFlags::from_iter(["\\Draft"]),
                object: Some(ReplicaHash("cafebabe".into())),
                meta: Some(ReplicaMeta("{\"subject\":\"hi\",\"v\":1}".into())),
                handle: Some(ReplicaHandle("draft-1".into())),
            },
            PimdirAction::Add {
                link_id: None,
                flags: ReplicaFlags::default(),
                object: None,
                meta: None,
                handle: None,
            },
            PimdirAction::SetFlags {
                seq: 4,
                flags: ReplicaFlags::from_iter(["\\Seen", "$flagged"]),
            },
            PimdirAction::Remove { seq: 5 },
            PimdirAction::Move {
                seq: 6,
                to: ReplicaCollectionId("Archive".into()),
            },
            PimdirAction::Copy {
                seq: 7,
                to: ReplicaCollectionId("Backup".into()),
            },
            PimdirAction::Update {
                seq: 8,
                object: ReplicaHash("beef0000".into()),
                meta: None,
            },
        ];
        for action in actions {
            let payload = action_to_payload(&action);
            // NOTE: versioned with a leading `v` (spec §15.3).
            assert!(payload.contains("\"v\":1"), "versioned: {payload}");
            let decoded = action_from_payload(action.kind(), &payload).unwrap();
            assert_eq!(decoded, action, "round-trip of {payload}");
        }
    }

    #[test]
    fn a_non_json_meta_survives_the_payload_embedding() {
        let action = PimdirAction::Update {
            seq: 1,
            object: ReplicaHash("cafebabe".into()),
            meta: Some(ReplicaMeta("not json".into())),
        };
        let payload = action_to_payload(&action);
        assert_eq!(action_from_payload("update", &payload).unwrap(), action);
    }

    #[test]
    fn malformed_action_payloads_error_instead_of_decaying() {
        // NOTE: strict, unlike the column decoders: the owner parks these.
        assert_eq!(
            action_from_payload("remove", "not json"),
            Err(PimdirActionError::Json)
        );
        assert_eq!(
            action_from_payload("remove", "{\"seq\":1}"),
            Err(PimdirActionError::UnknownVersion(None))
        );
        assert_eq!(
            action_from_payload("remove", "{\"v\":2,\"seq\":1}"),
            Err(PimdirActionError::UnknownVersion(Some(2)))
        );
        assert_eq!(
            action_from_payload("remove", "{\"v\":1}"),
            Err(PimdirActionError::MissingField("seq"))
        );
    }

    #[test]
    fn an_owner_defined_kind_survives_whole_instead_of_erroring() {
        // An intent only its owner can perform (a mail submission): this crate
        // keeps it verbatim rather than parking the row, and still pins the body
        // the payload references.
        let payload = "{\"v\":1,\"object\":\"cafebabe\",\"to\":[\"a@b.c\"]}";
        let decoded = action_from_payload("submit", payload).unwrap();
        assert_eq!(
            decoded,
            PimdirAction::Unknown {
                kind: "submit".into(),
                payload: payload.into(),
                object_hash: Some(ReplicaHash("cafebabe".into())),
            }
        );
        assert_eq!(decoded.kind(), "submit");
        assert_eq!(decoded.object_hash(), Some(&ReplicaHash("cafebabe".into())));
        // Byte-for-byte: nothing here understands the shape well enough to
        // re-encode it.
        assert_eq!(action_to_payload(&decoded), payload);

        // A malformed payload still parks, whatever its kind.
        assert_eq!(
            action_from_payload("submit", "{\"to\":[]}"),
            Err(PimdirActionError::UnknownVersion(None))
        );
        assert_eq!(
            action_from_payload("submit", "nope"),
            Err(PimdirActionError::Json)
        );
    }

    #[test]
    fn the_pinned_hash_follows_the_payload_body() {
        let add = PimdirAction::Add {
            link_id: None,
            flags: ReplicaFlags::default(),
            object: Some(ReplicaHash("cafebabe".into())),
            meta: None,
            handle: None,
        };
        assert_eq!(add.object_hash(), Some(&ReplicaHash("cafebabe".into())));
        assert_eq!(PimdirAction::Remove { seq: 1 }.object_hash(), None);
    }
}
