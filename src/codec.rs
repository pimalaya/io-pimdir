//! # Encodings
//!
//! The I/O-free encodings between the model and the pimdir columns
//! (STORAGE §13), and the action queue's versioned payload (STORAGE
//! §15.3): the part an implementation holding its own SQLite binding
//! reuses.

use core::fmt;

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use serde_json::{Map, Value, json};

use crate::{
    collection::PimdirCollectionId,
    hub::PimdirHubConflict,
    object::PimdirHash,
    placement::{PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId},
};

/// A flag set as its canonical JSON array, or `None` for the column's
/// `NULL`: a known-empty set is `"[]"`, a set nobody has read is `NULL`.
pub fn flags_to_json(flags: &PimdirFlags) -> Option<String> {
    let items: Vec<&String> = flags.known()?.iter().collect();
    Some(serde_json::to_string(&items).unwrap_or_else(|_| String::from("[]")))
}

/// The inverse of [`flags_to_json`]: `NULL` and a column this cannot read
/// both decode to the unknown set, which holds no opinion in a merge.
pub fn flags_from_json(json: Option<&str>) -> PimdirFlags {
    let Some(json) = json else {
        return PimdirFlags::Unknown;
    };
    match serde_json::from_str::<Vec<String>>(json) {
        Ok(items) => PimdirFlags::Known(items.into_iter().collect()),
        Err(_) => PimdirFlags::Unknown,
    }
}

/// A list of ids as the JSON array column `in_reply_to` holds.
pub fn ids_to_json(ids: &[String]) -> String {
    serde_json::to_string(ids).unwrap_or_else(|_| String::from("[]"))
}

/// The inverse of [`ids_to_json`]; an unreadable column is an empty list.
pub fn ids_from_json(json: &str) -> Vec<String> {
    serde_json::from_str(json).unwrap_or_default()
}

/// The detail ladder as its column integer.
pub fn level_to_int(level: PimdirLevel) -> i64 {
    match level {
        PimdirLevel::Probed => 0,
        PimdirLevel::Meta => 1,
        PimdirLevel::Full => 2,
    }
}

/// The inverse of [`level_to_int`]; an unknown integer clamps to `Probed`.
pub fn level_from_int(value: i64) -> PimdirLevel {
    match value {
        1 => PimdirLevel::Meta,
        2 => PimdirLevel::Full,
        _ => PimdirLevel::Probed,
    }
}

/// A collection's cross-source conflict policy as its column spelling.
pub fn conflict_to_str(policy: PimdirHubConflict) -> &'static str {
    match policy {
        PimdirHubConflict::Manual => "manual",
        PimdirHubConflict::PreferIncoming => "prefer-incoming",
        PimdirHubConflict::PreferExisting => "prefer-existing",
    }
}

/// The inverse of [`conflict_to_str`]; an unknown spelling is `Manual`.
pub fn conflict_from_str(value: &str) -> PimdirHubConflict {
    match value {
        "prefer-incoming" => PimdirHubConflict::PreferIncoming,
        "prefer-existing" => PimdirHubConflict::PreferExisting,
        _ => PimdirHubConflict::Manual,
    }
}

/// A queued mutation request (STORAGE §15.3): what a producer appends and
/// the owner applies, existing items addressed by their public `seq`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirAction {
    /// Create an item, staged as a local creation for the sync to push.
    ///
    /// The owner derives the summary and addresses from the body. A
    /// duplicate live `link_id` parks the action, a retained one revives.
    Add {
        /// The item's key; `None` derives it from the body.
        link_id: Option<PimdirLinkId>,
        /// The initial flag set.
        flags: PimdirFlags,
        /// The body's hash, written durably by the producer before enqueueing.
        object: Option<PimdirHash>,
        /// The provisional handle the create is staged under.
        handle: Option<PimdirHandle>,
    },
    /// Replace the item's flag set, absolutely.
    SetFlags {
        /// The item's public id.
        seq: i64,
        /// The new flag set.
        flags: PimdirFlags,
    },
    /// Remove the item from the collection; already absent is success.
    Remove {
        /// The item's public id.
        seq: i64,
    },
    /// Refile the item into another collection.
    Move {
        /// The item's public id.
        seq: i64,
        /// The collection to move it into.
        to: PimdirCollectionId,
    },
    /// Copy the item into another collection.
    Copy {
        /// The item's public id.
        seq: i64,
        /// The collection to copy it into.
        to: PimdirCollectionId,
    },
    /// Repoint a mutable-content item's body; the owner re-derives its summary.
    Update {
        /// The item's public id.
        seq: i64,
        /// The new body's hash, written durably by the producer before enqueueing.
        object: PimdirHash,
    },
    /// An application's own intent, which the store skips rather than parks.
    ///
    /// The owner that recognises the kind performs it out of band and
    /// acknowledges it by dropping the row.
    Unknown {
        /// The raw `queue.action` kind.
        kind: String,
        /// The raw versioned JSON payload, verbatim.
        payload: String,
        /// The body the payload's `object` field pins, by the shared convention.
        object_hash: Option<PimdirHash>,
    },
}

impl PimdirAction {
    /// The action kind as its `queue.action` column value.
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

    /// The body the payload references, which the enqueue pins (§15.1).
    pub fn object_hash(&self) -> Option<&PimdirHash> {
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

/// A malformed action payload, which the owner parks the row with.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirActionError {
    /// The payload is not a JSON object.
    Json,
    /// The payload's leading `v` is missing or unsupported.
    UnknownVersion(Option<i64>),
    /// A required field is missing or has the wrong shape.
    MissingField(&'static str),
}

impl fmt::Display for PimdirActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json => write!(f, "Pimdir action payload is not a JSON object"),
            Self::UnknownVersion(Some(v)) => write!(f, "Unknown pimdir action version: {v}"),
            Self::UnknownVersion(None) => write!(f, "Pimdir action payload misses its version"),
            Self::MissingField(field) => {
                write!(f, "Pimdir action payload misses field: {field}")
            }
        }
    }
}

impl core::error::Error for PimdirActionError {}

/// Encodes an action to its versioned JSON payload (`v: 1`).
pub fn action_to_payload(action: &PimdirAction) -> String {
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
            handle,
        } => {
            if let Some(link) = link_id {
                map.insert("link_id".into(), json!(link.0));
            }
            map.insert("flags".into(), flags_to_value(flags));
            if let Some(object) = object {
                map.insert("object".into(), json!(object.0));
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
        PimdirAction::Update { seq, object } => {
            map.insert("seq".into(), json!(seq));
            map.insert("object".into(), json!(object.0));
        }
        PimdirAction::Unknown { .. } => {}
    }

    Value::Object(map).to_string()
}

/// Decodes a kind and its payload, strictly: a malformed payload is an
/// error the owner parks with, an unknown kind an intent it skips.
pub fn action_from_payload(kind: &str, payload: &str) -> Result<PimdirAction, PimdirActionError> {
    let value: Value = serde_json::from_str(payload).map_err(|_| PimdirActionError::Json)?;
    let map = value.as_object().ok_or(PimdirActionError::Json)?;

    let version = map.get("v").and_then(Value::as_i64);
    if version != Some(1) {
        return Err(PimdirActionError::UnknownVersion(version));
    }

    match kind {
        "add" => Ok(PimdirAction::Add {
            link_id: get_string(map, "link_id")?.map(PimdirLinkId),
            flags: flags_from_value(map.get("flags")),
            object: get_string(map, "object")?.map(PimdirHash),
            handle: get_string(map, "handle")?.map(PimdirHandle),
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
            to: PimdirCollectionId(require_string(map, "to")?),
        }),
        "copy" => Ok(PimdirAction::Copy {
            seq: require_seq(map)?,
            to: PimdirCollectionId(require_string(map, "to")?),
        }),
        "update" => Ok(PimdirAction::Update {
            seq: require_seq(map)?,
            object: PimdirHash(require_string(map, "object")?),
        }),
        other => Ok(PimdirAction::Unknown {
            kind: other.to_string(),
            payload: payload.to_string(),
            object_hash: get_string(map, "object")?.map(PimdirHash),
        }),
    }
}

/// A flag set as a JSON array, `null` for an unknown one: an action states
/// an intent, and an unknown set must not read as clearing every flag.
fn flags_to_value(flags: &PimdirFlags) -> Value {
    match flags.known() {
        None => Value::Null,
        Some(flags) => Value::Array(flags.iter().map(|f| json!(f)).collect()),
    }
}

fn flags_from_value(value: Option<&Value>) -> PimdirFlags {
    match value {
        None | Some(Value::Null) => PimdirFlags::Unknown,
        Some(value) => PimdirFlags::Known(
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

fn require_string(
    map: &Map<String, Value>,
    field: &'static str,
) -> Result<String, PimdirActionError> {
    get_string(map, field)?.ok_or(PimdirActionError::MissingField(field))
}

fn require_seq(map: &Map<String, Value>) -> Result<i64, PimdirActionError> {
    map.get("seq")
        .and_then(Value::as_i64)
        .ok_or(PimdirActionError::MissingField("seq"))
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test]
    fn a_flag_set_round_trips_and_keeps_its_two_absences_apart() {
        let flags = PimdirFlags::from_iter(["\\Seen", "$flagged", "a\"b"]);
        let json = flags_to_json(&flags).expect("a known set encodes");
        assert_eq!(flags_from_json(Some(&json)), flags);
        assert!(json.contains("\\\\Seen"));

        assert_eq!(flags_to_json(&PimdirFlags::Unknown), None);
        assert_eq!(flags_from_json(None), PimdirFlags::Unknown);
        assert_eq!(
            flags_to_json(&PimdirFlags::default()).as_deref(),
            Some("[]")
        );
        assert_eq!(flags_from_json(Some("[]")), PimdirFlags::default());
        assert_eq!(flags_from_json(Some("not json")), PimdirFlags::Unknown);
    }

    #[test]
    fn levels_and_policies_round_trip() {
        for level in [PimdirLevel::Probed, PimdirLevel::Meta, PimdirLevel::Full] {
            assert_eq!(level_from_int(level_to_int(level)), level);
        }
        for policy in [
            PimdirHubConflict::Manual,
            PimdirHubConflict::PreferIncoming,
            PimdirHubConflict::PreferExisting,
        ] {
            assert_eq!(conflict_from_str(conflict_to_str(policy)), policy);
        }
        assert_eq!(
            ids_from_json(&ids_to_json(&["a".into(), "b".into()])),
            vec!["a", "b"]
        );
    }

    #[test]
    fn every_action_kind_round_trips_through_its_payload() {
        let actions = [
            PimdirAction::Add {
                link_id: Some(PimdirLinkId("mid:new".into())),
                flags: PimdirFlags::from_iter(["\\Draft"]),
                object: Some(PimdirHash("cafebabe".into())),
                handle: Some(PimdirHandle("draft-1".into())),
            },
            PimdirAction::Add {
                link_id: None,
                flags: PimdirFlags::default(),
                object: None,
                handle: None,
            },
            PimdirAction::SetFlags {
                seq: 4,
                flags: PimdirFlags::from_iter(["\\Seen", "$flagged"]),
            },
            PimdirAction::Remove { seq: 5 },
            PimdirAction::Move {
                seq: 6,
                to: PimdirCollectionId("Archive".into()),
            },
            PimdirAction::Copy {
                seq: 7,
                to: PimdirCollectionId("Backup".into()),
            },
            PimdirAction::Update {
                seq: 8,
                object: PimdirHash("beef0000".into()),
            },
        ];
        for action in actions {
            let payload = action_to_payload(&action);
            assert!(payload.contains("\"v\":1"), "versioned: {payload}");
            let decoded = action_from_payload(action.kind(), &payload).unwrap();
            assert_eq!(decoded, action, "round-trip of {payload}");
        }
    }

    #[test]
    fn a_malformed_payload_errors_and_a_foreign_kind_survives_whole() {
        assert_eq!(
            action_from_payload("remove", "not json"),
            Err(PimdirActionError::Json)
        );
        assert_eq!(
            action_from_payload("remove", "{\"v\":2,\"seq\":1}"),
            Err(PimdirActionError::UnknownVersion(Some(2)))
        );
        assert_eq!(
            action_from_payload("remove", "{\"v\":1}"),
            Err(PimdirActionError::MissingField("seq"))
        );

        let payload = "{\"v\":1,\"object\":\"cafebabe\",\"to\":[\"a@b.c\"]}";
        let decoded = action_from_payload("submit", payload).unwrap();
        assert_eq!(decoded.kind(), "submit");
        assert_eq!(decoded.object_hash(), Some(&PimdirHash("cafebabe".into())));
        assert_eq!(action_to_payload(&decoded), payload);
    }
}
