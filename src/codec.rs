//! # Encodings
//!
//! The I/O-free encodings between the model and the pimdir columns
//! (STORAGE §13), and the action queue's versioned payload (STORAGE
//! §15.3): the part an implementation holding its own SQLite binding
//! reuses.

use core::fmt;

use alloc::{
    collections::BTreeSet,
    format,
    string::{String, ToString},
    vec::Vec,
};

use serde_json::{Map, Value, json};

use crate::{
    capability,
    collection::PimdirCollectionId,
    hub::PimdirHubConflict,
    intent::{self, PimdirCollectionCreate, PimdirInvitation, PimdirSubmit},
    object::PimdirHash,
    placement::{PimdirFlags, PimdirLevel, PimdirLinkId},
};

/// A flag set as a read statement answers it (STORAGE §4.4, §13): the
/// flags joined by single spaces in no order, `''` the known-empty set and
/// `NULL` the unknown one, which holds no opinion in a merge.
pub fn flags_from_column(joined: Option<&str>) -> PimdirFlags {
    let Some(joined) = joined else {
        return PimdirFlags::Unknown;
    };
    PimdirFlags::Known(
        joined
            .split(' ')
            .filter(|flag| !flag.is_empty())
            .map(String::from)
            .collect(),
    )
}

/// Whether the store holds `flag` (STORAGE §13): non-empty and printable
/// ASCII without a space, as IMAP flags and JMAP keywords are. A set
/// holding another is refused before it is written.
pub fn is_flag(flag: &str) -> bool {
    !flag.is_empty() && flag.bytes().all(|byte| matches!(byte, b'!'..=b'~'))
}

/// The detail ladder as its column integer.
pub fn level_to_int(level: PimdirLevel) -> i64 {
    match level {
        PimdirLevel::Meta => 1,
        PimdirLevel::Full => 2,
    }
}

/// The inverse of [`level_to_int`]: `2` is `Full` and `1` `Meta`, the
/// only two the schema holds (STORAGE §13).
pub fn level_from_int(value: i64) -> PimdirLevel {
    match value {
        2 => PimdirLevel::Full,
        _ => PimdirLevel::Meta,
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

/// The item an `add` replaces (STORAGE §15.3): a task filed into another
/// collection, renamed or not, rather than moved there.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirSupersedes {
    /// The collection holding it.
    pub collection: PimdirCollectionId,
    /// Its public id there.
    pub seq: i64,
}

/// A queued mutation request (STORAGE §15.3).
///
/// What a producer appends and the owner applies, existing items
/// addressed by their public `seq`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirAction {
    /// Create an item, staged as a local creation for the sync to push.
    ///
    /// The owner derives the summary and addresses from the body. A
    /// duplicate live `link_id` parks the action, a retained one revives.
    Add {
        /// The item's key; `None` derives it from the body.
        link_id: Option<PimdirLinkId>,
        /// The initial flag set, known: an unknown one is no payload.
        flags: PimdirFlags,
        /// The body's hash, written durably by the producer before
        /// enqueueing; a payload requires it, so `None` is refused there.
        object: Option<PimdirHash>,
        /// The item this one replaces (§15.3): applied with its tombstone in
        /// one transaction, its delete held until this add is pushed.
        supersedes: Option<PimdirSupersedes>,
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
        /// The body the producer read and replaced (§15.3): the owner parks
        /// the row when the item holds another, a write it would otherwise
        /// drop without a merge; `None` applies over whatever it holds.
        if_match: Option<PimdirHash>,
        /// The diverging bodies a person's resolution was decided over
        /// (§15.3), the `conflict_object` of each conflict the producer
        /// showed, at least one and each once: the update settles the
        /// conflicts it meets only when it names each, and parks when it
        /// meets none. `None` for an ordinary edit, which parks over any
        /// conflict.
        resolves: Option<Vec<PimdirHash>>,
    },
    /// Record the source performing an intent capability for the account
    /// of the collection the action is anchored on (§15.6), or withdraw
    /// the choice when `source` is `None`.
    SetPerformer {
        /// The intent capability (Annex B.2).
        capability: String,
        /// The source the user chose.
        source: Option<String>,
    },
    /// An intent, of Annex B.2 or an application's own, which the store
    /// skips rather than applies.
    ///
    /// The owner that recognises the kind performs it out of band and
    /// acknowledges it by dropping the row. An Annex B.2 intent is read
    /// strictly by its type in [`crate::intent`], a malformed one parked.
    Unknown {
        /// The raw `queue.action` kind.
        kind: String,
        /// The raw versioned JSON payload, verbatim.
        payload: String,
        /// The body the row pins, its `queue.object_hash` (§15.3): what a
        /// producer building the action and the queue reads set, `None`
        /// from [`action_from_payload`], which reads the payload alone.
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
            Self::SetPerformer { .. } => "set-performer",
            Self::Unknown { kind, .. } => kind,
        }
    }

    /// The collection the action names beside its own (§15.3), which the
    /// producer writes as `queue.target_collection`: a `move` or a
    /// `copy`'s `to`, a superseding `add`'s `supersedes.collection`, `None`
    /// for any other action.
    pub fn target_collection(&self) -> Option<&PimdirCollectionId> {
        match self {
            Self::Move { to, .. } | Self::Copy { to, .. } => Some(to),
            Self::Add {
                supersedes: Some(supersedes),
                ..
            } => Some(&supersedes.collection),
            _ => None,
        }
    }

    /// Points the action at `target`, the `queue.target_collection` a
    /// rename followed (§14): the owner reads it in place of the payload's
    /// own. An action naming no collection beside its own is left as it is.
    pub fn retarget(&mut self, target: PimdirCollectionId) {
        match self {
            Self::Move { to, .. } | Self::Copy { to, .. } => *to = target,
            Self::Add {
                supersedes: Some(supersedes),
                ..
            } => supersedes.collection = target,
            _ => {}
        }
    }

    /// The body the payload references, which the enqueue pins (§15.1).
    pub fn object_hash(&self) -> Option<&PimdirHash> {
        match self {
            Self::Add { object, .. } => object.as_ref(),
            Self::Update { object, .. } => Some(object),
            Self::Unknown { object_hash, .. } => object_hash.as_ref(),
            Self::SetFlags { .. }
            | Self::Remove { .. }
            | Self::Move { .. }
            | Self::Copy { .. }
            | Self::SetPerformer { .. } => None,
        }
    }
}

/// A malformed action payload (STORAGE §15.3), which a producer refuses
/// at the enqueue and the owner parks the row with, naming the member.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirActionError {
    /// The payload is not a JSON object.
    Json,
    /// The payload's leading `v` is missing or unsupported.
    UnknownVersion(Option<i64>),
    /// A member the payload's schema requires is missing.
    MissingField(&'static str),
    /// A member the payload's version does not define, by its name: a
    /// payload changing shape takes the next `v`.
    UnknownField(String),
    /// A member is of another type than its schema states, or breaks a
    /// rule of its action or intent (Annex B.2).
    Invalid {
        /// The member.
        field: &'static str,
        /// The rule it breaks.
        reason: &'static str,
    },
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
            Self::UnknownField(field) => {
                write!(
                    f,
                    "Pimdir action payload holds field {field}, which its version does not define"
                )
            }
            Self::Invalid { field, reason } => {
                write!(
                    f,
                    "Pimdir action payload field {field} is invalid: {reason}"
                )
            }
        }
    }
}

impl core::error::Error for PimdirActionError {}

/// Encodes an action to its versioned JSON payload (`v: 1`).
///
/// What the action holds is written as it is: an `add` with no `object`
/// or an unknown flag set encodes to a payload [`action_from_payload`]
/// refuses, which is how a producer refuses it at the enqueue.
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
            supersedes,
        } => {
            if let Some(link) = link_id {
                map.insert("link_id".into(), json!(link.0));
            }
            map.insert("flags".into(), flags_to_value(flags));
            if let Some(object) = object {
                map.insert("object".into(), json!(object.0));
            }
            if let Some(supersedes) = supersedes {
                map.insert(
                    "supersedes".into(),
                    json!({ "collection": supersedes.collection.0, "seq": supersedes.seq }),
                );
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
        PimdirAction::Update {
            seq,
            object,
            if_match,
            resolves,
        } => {
            map.insert("seq".into(), json!(seq));
            map.insert("object".into(), json!(object.0));
            if let Some(if_match) = if_match {
                map.insert("if_match".into(), json!(if_match.0));
            }
            if let Some(resolves) = resolves {
                let names: Vec<&str> = resolves.iter().map(PimdirHash::as_str).collect();
                map.insert("resolves".into(), json!(names));
            }
        }
        PimdirAction::SetPerformer { capability, source } => {
            map.insert("capability".into(), json!(capability));
            if let Some(source) = source {
                map.insert("source".into(), json!(source));
            }
        }
        PimdirAction::Unknown { .. } => {}
    }

    Value::Object(map).to_string()
}

/// Decodes a kind and its payload strictly, as payloads/ types it at
/// `v: 1` (STORAGE §15.3): a member missing, of another type, breaking its
/// rule or one the version does not define is an error the owner parks
/// with and a producer refuses at the enqueue, a version it does not know
/// one it skips.
///
/// An intent of Annex B.2 is read whole by its type in [`crate::intent`]
/// and kept as [`PimdirAction::Unknown`], as is an application's own kind,
/// read as a JSON object alone. Neither names its body here: that is the
/// row's `queue.object_hash`, which the queue reads fill in.
pub fn action_from_payload(kind: &str, payload: &str) -> Result<PimdirAction, PimdirActionError> {
    let action = match kind {
        "add" => {
            let map = members(payload, &["link_id", "flags", "object", "supersedes"])?;
            PimdirAction::Add {
                link_id: label(map.get("link_id"), "link_id")?.map(PimdirLinkId),
                flags: required(flags(map.get("flags"), "flags")?, "flags")?,
                object: Some(required(hash(map.get("object"), "object")?, "object")?),
                supersedes: supersedes(map.get("supersedes"))?,
            }
        }
        "set-flags" => {
            let map = members(payload, &["seq", "flags"])?;
            PimdirAction::SetFlags {
                seq: required(seq(map.get("seq"), "seq")?, "seq")?,
                flags: required(flags(map.get("flags"), "flags")?, "flags")?,
            }
        }
        "remove" => {
            let map = members(payload, &["seq"])?;
            PimdirAction::Remove {
                seq: required(seq(map.get("seq"), "seq")?, "seq")?,
            }
        }
        "move" | "copy" => {
            let map = members(payload, &["seq", "to"])?;
            let seq = required(seq(map.get("seq"), "seq")?, "seq")?;
            let to = PimdirCollectionId(required(label(map.get("to"), "to")?, "to")?);
            match kind {
                "move" => PimdirAction::Move { seq, to },
                _ => PimdirAction::Copy { seq, to },
            }
        }
        "update" => {
            let map = members(payload, &["seq", "object", "if_match", "resolves"])?;
            PimdirAction::Update {
                seq: required(seq(map.get("seq"), "seq")?, "seq")?,
                object: required(hash(map.get("object"), "object")?, "object")?,
                if_match: hash(map.get("if_match"), "if_match")?,
                resolves: hashes(map.get("resolves"), "resolves")?,
            }
        }
        "set-performer" => {
            let map = members(payload, &["capability", "source"])?;
            let capability = required(text(map.get("capability"), "capability")?, "capability")?;
            if !capability::is_name(&capability) {
                return Err(PimdirActionError::Invalid {
                    field: "capability",
                    reason: "a capability is lowercase words of letters, digits and hyphens joined by periods",
                });
            }
            PimdirAction::SetPerformer {
                capability,
                source: label(map.get("source"), "source")?,
            }
        }
        _ => {
            // NOTE: read for its errors alone, the payload kept verbatim.
            match kind {
                intent::SUBMIT => {
                    PimdirSubmit::from_payload(payload)?;
                }
                intent::CALENDAR_REPLY | intent::CALENDAR_CANCEL => {
                    PimdirInvitation::from_payload(kind, payload)?;
                }
                intent::COLLECTION_CREATE => {
                    PimdirCollectionCreate::from_payload(payload)?;
                }
                _ => {
                    members_any(payload)?;
                }
            }
            PimdirAction::Unknown {
                kind: kind.to_string(),
                payload: payload.to_string(),
                object_hash: None,
            }
        }
    };

    Ok(action)
}

/// A flag set as a JSON array, `null` for an unknown one, which no payload
/// at `v: 1` holds: an unknown set must not read as clearing every flag,
/// so an action carrying one is refused rather than queued.
fn flags_to_value(flags: &PimdirFlags) -> Value {
    match flags.known() {
        None => Value::Null,
        Some(flags) => Value::Array(flags.iter().map(|f| json!(f)).collect()),
    }
}

/// The members of a payload at `v: 1`, each one of `defined` or `v`: a
/// payload that is no JSON object, states another version or holds a
/// member its version does not define is refused, the version first, so
/// a later producer's payload is skipped rather than parked (§15.2).
pub(crate) fn members(
    payload: &str,
    defined: &[&str],
) -> Result<Map<String, Value>, PimdirActionError> {
    let map = members_any(payload)?;

    let undefined = map
        .keys()
        .find(|member| *member != "v" && !defined.contains(&member.as_str()));
    if let Some(member) = undefined {
        return Err(PimdirActionError::UnknownField(member.clone()));
    }

    Ok(map)
}

/// The members of a payload at `v: 1`, whatever they are: an
/// application's own kind, whose schema the store does not know.
fn members_any(payload: &str) -> Result<Map<String, Value>, PimdirActionError> {
    let value: Value = serde_json::from_str(payload).map_err(|_| PimdirActionError::Json)?;
    let Value::Object(map) = value else {
        return Err(PimdirActionError::Json);
    };

    let version = map.get("v").and_then(Value::as_i64);
    if version != Some(1) {
        return Err(PimdirActionError::UnknownVersion(version));
    }

    Ok(map)
}

/// A member the schema requires.
pub(crate) fn required<T>(value: Option<T>, field: &'static str) -> Result<T, PimdirActionError> {
    value.ok_or(PimdirActionError::MissingField(field))
}

/// A `text` member: any string.
pub(crate) fn text(
    value: Option<&Value>,
    field: &'static str,
) -> Result<Option<String>, PimdirActionError> {
    match value {
        None => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(PimdirActionError::Invalid {
            field,
            reason: "a string is expected",
        }),
    }
}

/// A `key`, `id`, `collection`, `addr` or `label` member: a non-empty
/// string.
pub(crate) fn label(
    value: Option<&Value>,
    field: &'static str,
) -> Result<Option<String>, PimdirActionError> {
    let label = text(value, field)?;
    if label.as_deref() == Some("") {
        return Err(PimdirActionError::Invalid {
            field,
            reason: "a non-empty string is expected",
        });
    }
    Ok(label)
}

/// An `n` member: an integer from 1, the public id it names.
pub(crate) fn seq(
    value: Option<&Value>,
    field: &'static str,
) -> Result<Option<i64>, PimdirActionError> {
    match value {
        None => Ok(None),
        Some(value) => match value.as_i64() {
            Some(seq) if seq >= 1 => Ok(Some(seq)),
            _ => Err(PimdirActionError::Invalid {
                field,
                reason: "an integer from 1 is expected",
            }),
        },
    }
}

/// A `hash` member: an object name (§5), never a path.
fn hash(
    value: Option<&Value>,
    field: &'static str,
) -> Result<Option<PimdirHash>, PimdirActionError> {
    let Some(hash) = text(value, field)?.map(PimdirHash) else {
        return Ok(None);
    };
    if !hash.is_name() {
        return Err(PimdirActionError::Invalid {
            field,
            reason: "an object name is expected, 52 characters of a to z and 2 to 7",
        });
    }
    Ok(Some(hash))
}

/// A `[hash…]` member: a non-empty array of distinct object names (§15.3).
fn hashes(
    value: Option<&Value>,
    field: &'static str,
) -> Result<Option<Vec<PimdirHash>>, PimdirActionError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let invalid = PimdirActionError::Invalid {
        field,
        reason: "a non-empty array of distinct object names is expected",
    };
    let Some(items) = value.as_array().filter(|items| !items.is_empty()) else {
        return Err(invalid);
    };

    let mut names: Vec<PimdirHash> = Vec::with_capacity(items.len());
    for item in items {
        let name = required(hash(Some(item), field)?, field)?;
        if names.contains(&name) {
            return Err(invalid);
        }
        names.push(name);
    }
    Ok(Some(names))
}

/// A `[…]` member: a flag set as distinct flags, each one the store holds
/// (§13).
fn flags(
    value: Option<&Value>,
    field: &'static str,
) -> Result<Option<PimdirFlags>, PimdirActionError> {
    let invalid = PimdirActionError::Invalid {
        field,
        reason: "an array of distinct flags of printable ASCII is expected",
    };
    let Some(value) = value else {
        return Ok(None);
    };
    let Value::Array(items) = value else {
        return Err(invalid);
    };

    let mut flags = BTreeSet::new();
    for item in items {
        match item.as_str() {
            Some(flag) if is_flag(flag) && flags.insert(flag.to_string()) => {}
            _ => return Err(invalid),
        }
    }

    Ok(Some(PimdirFlags::Known(flags)))
}

/// An `add`'s `supersedes`: absent is none, else an object naming exactly
/// a collection and a seq.
fn supersedes(value: Option<&Value>) -> Result<Option<PimdirSupersedes>, PimdirActionError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Value::Object(inner) = value else {
        return Err(PimdirActionError::Invalid {
            field: "supersedes",
            reason: "an object naming a collection and a seq is expected",
        });
    };

    let undefined = inner
        .keys()
        .find(|member| !matches!(member.as_str(), "collection" | "seq"));
    if let Some(member) = undefined {
        return Err(PimdirActionError::UnknownField(format!(
            "supersedes.{member}"
        )));
    }

    let collection = label(inner.get("collection"), "supersedes.collection")?;
    let seq = seq(inner.get("seq"), "supersedes.seq")?;
    Ok(Some(PimdirSupersedes {
        collection: PimdirCollectionId(required(collection, "supersedes.collection")?),
        seq: required(seq, "supersedes.seq")?,
    }))
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test]
    fn a_joined_flag_set_reads_in_no_order_and_keeps_its_two_absences_apart() {
        assert_eq!(
            flags_from_column(Some("$flagged \\Seen")),
            flags_from_column(Some("\\Seen $flagged")),
        );
        assert!(flags_from_column(Some("$flagged \\Seen")).contains("\\Seen"));
        assert_eq!(flags_from_column(Some("")), PimdirFlags::default());
        assert_eq!(flags_from_column(None), PimdirFlags::Unknown);
    }

    #[test]
    fn a_flag_is_printable_ascii_without_a_space() {
        for flag in ["\\Seen", "$label", "a\"b", "!", "~"] {
            assert!(is_flag(flag), "{flag:?}");
        }
        for flag in ["", "two words", "tab\t", "\u{7f}", "café", "\n"] {
            assert!(!is_flag(flag), "{flag:?}");
        }
    }

    #[test]
    fn a_move_a_copy_and_a_superseding_add_carry_their_target() {
        let to = PimdirCollectionId("Archive".into());
        let mut moved = PimdirAction::Move {
            seq: 1,
            to: PimdirCollectionId("Old".into()),
        };
        moved.retarget(to.clone());
        assert_eq!(moved.target_collection(), Some(&to));

        let mut added = PimdirAction::Add {
            link_id: None,
            flags: PimdirFlags::default(),
            object: None,
            supersedes: Some(PimdirSupersedes {
                collection: PimdirCollectionId("Old".into()),
                seq: 2,
            }),
        };
        added.retarget(to.clone());
        assert_eq!(added.target_collection(), Some(&to));

        let mut removed = PimdirAction::Remove { seq: 3 };
        removed.retarget(to);
        assert_eq!(removed.target_collection(), None);
        assert_eq!(removed, PimdirAction::Remove { seq: 3 });
    }

    #[test]
    fn levels_and_policies_round_trip() {
        for level in [PimdirLevel::Meta, PimdirLevel::Full] {
            assert_eq!(level_from_int(level_to_int(level)), level);
        }
        for policy in [
            PimdirHubConflict::Manual,
            PimdirHubConflict::PreferIncoming,
            PimdirHubConflict::PreferExisting,
        ] {
            assert_eq!(conflict_from_str(conflict_to_str(policy)), policy);
        }
    }

    /// The object names of the empty body and of `len-1` (objects.json).
    const EMPTY: &str = "v4jutopv7gq2nicajxvdnxgjjgn4wjojvxarfn6mtkj4vza7gjra";
    const LEN_1: &str = "fu5n5x7rdnq7cteiny227ibwonw43b5hjut3lqkrais5b5ms4ijq";

    #[test]
    fn a_malformed_supersedes_is_refused_and_an_absent_one_is_none() {
        let refused = |supersedes: &str| {
            let payload =
                format!(r#"{{"v":1,"flags":[],"object":"{EMPTY}","supersedes":{supersedes}}}"#);
            action_from_payload("add", &payload).unwrap_err()
        };
        assert!(matches!(
            refused(r#""inbox""#),
            PimdirActionError::Invalid {
                field: "supersedes",
                ..
            }
        ));
        assert_eq!(
            refused(r#"{"seq":3}"#),
            PimdirActionError::MissingField("supersedes.collection")
        );
        assert!(matches!(
            refused(r#"{"collection":"inbox","seq":"3"}"#),
            PimdirActionError::Invalid {
                field: "supersedes.seq",
                ..
            }
        ));
        assert_eq!(
            refused(r#"{"collection":"inbox","seq":3,"by":"me"}"#),
            PimdirActionError::UnknownField("supersedes.by".into())
        );

        let payload = format!(r#"{{"v":1,"flags":[],"object":"{EMPTY}"}}"#);
        let plain = action_from_payload("add", &payload).unwrap();
        assert!(matches!(
            plain,
            PimdirAction::Add {
                supersedes: None,
                ..
            }
        ));
    }

    #[test]
    fn every_action_kind_round_trips_through_its_payload() {
        let actions = [
            PimdirAction::Add {
                link_id: Some(PimdirLinkId("mid:new".into())),
                flags: PimdirFlags::from_iter(["\\Draft"]),
                object: Some(PimdirHash(EMPTY.into())),
                supersedes: None,
            },
            PimdirAction::Add {
                link_id: None,
                flags: PimdirFlags::default(),
                object: Some(PimdirHash(LEN_1.into())),
                supersedes: Some(PimdirSupersedes {
                    collection: PimdirCollectionId("Inbox".into()),
                    seq: 3,
                }),
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
                object: PimdirHash(LEN_1.into()),
                if_match: None,
                resolves: None,
            },
            PimdirAction::Update {
                seq: 9,
                object: PimdirHash(LEN_1.into()),
                if_match: Some(PimdirHash(EMPTY.into())),
                resolves: None,
            },
            PimdirAction::Update {
                seq: 9,
                object: PimdirHash(LEN_1.into()),
                if_match: Some(PimdirHash(EMPTY.into())),
                resolves: Some(vec![PimdirHash(EMPTY.into()), PimdirHash(LEN_1.into())]),
            },
            PimdirAction::SetPerformer {
                capability: "mail.submit".into(),
                source: Some("smtp".into()),
            },
        ];
        for action in actions {
            let payload = action_to_payload(&action);
            assert!(payload.contains("\"v\":1"), "versioned: {payload}");
            let decoded = action_from_payload(action.kind(), &payload).unwrap();
            assert_eq!(decoded, action, "round-trip of {payload}");
        }
    }

    /// What no payload at `v: 1` holds encodes to one the decode refuses,
    /// so a producer re-reading its own payload refuses the action.
    #[test]
    fn an_add_without_a_body_or_a_known_flag_set_encodes_to_a_refused_payload() {
        let refused = |action: PimdirAction| {
            action_from_payload(action.kind(), &action_to_payload(&action)).unwrap_err()
        };

        let bodiless = PimdirAction::Add {
            link_id: None,
            flags: PimdirFlags::default(),
            object: None,
            supersedes: None,
        };
        assert_eq!(refused(bodiless), PimdirActionError::MissingField("object"));

        let unread = PimdirAction::SetFlags {
            seq: 1,
            flags: PimdirFlags::Unknown,
        };
        assert!(matches!(
            refused(unread),
            PimdirActionError::Invalid { field: "flags", .. }
        ));
    }

    /// A payload is valid whole or malformed (§15.3), the error naming the
    /// member, and a version this codec does not know is told apart.
    #[test]
    fn a_malformed_payload_names_its_member() {
        let invalid = |field| PimdirActionError::Invalid { field, reason: "" };
        let cases = [
            ("remove", "not json".into(), PimdirActionError::Json),
            ("remove", "[1]".into(), PimdirActionError::Json),
            (
                "remove",
                r#"{"seq":1}"#.into(),
                PimdirActionError::UnknownVersion(None),
            ),
            (
                "remove",
                r#"{"v":2,"seq":1}"#.into(),
                PimdirActionError::UnknownVersion(Some(2)),
            ),
            (
                "remove",
                r#"{"v":1}"#.into(),
                PimdirActionError::MissingField("seq"),
            ),
            ("remove", r#"{"v":1,"seq":-3}"#.into(), invalid("seq")),
            ("remove", r#"{"v":1,"seq":1.5}"#.into(), invalid("seq")),
            ("move", r#"{"v":1,"seq":1,"to":null}"#.into(), invalid("to")),
            (
                "add",
                format!(r#"{{"v":1,"object":"{EMPTY}"}}"#),
                PimdirActionError::MissingField("flags"),
            ),
            (
                "add",
                format!(r#"{{"v":1,"flags":null,"object":"{EMPTY}"}}"#),
                invalid("flags"),
            ),
            (
                "add",
                format!(r#"{{"v":1,"flags":[1],"object":"{EMPTY}"}}"#),
                invalid("flags"),
            ),
            (
                "add",
                format!(r#"{{"v":1,"flags":[""],"object":"{EMPTY}"}}"#),
                invalid("flags"),
            ),
            (
                "add",
                format!(r#"{{"v":1,"flags":[],"object":"{EMPTY}","link_id":""}}"#),
                invalid("link_id"),
            ),
            (
                "add",
                r#"{"v":1,"flags":[],"object":"objects/v4/v4"}"#.into(),
                invalid("object"),
            ),
            (
                "update",
                format!(r#"{{"v":1,"seq":1,"object":"{EMPTY}","if_match":7}}"#),
                invalid("if_match"),
            ),
            (
                "set-flags",
                r#"{"v":1,"seq":1,"flags":[],"why":"x"}"#.into(),
                PimdirActionError::UnknownField("why".into()),
            ),
            (
                "set-performer",
                r#"{"v":1,"capability":"mail..submit"}"#.into(),
                invalid("capability"),
            ),
            (
                "set-performer",
                r#"{"v":1,"capability":"mail.submit","source":""}"#.into(),
                invalid("source"),
            ),
            (
                "x-fax-send",
                r#"{"to":"+33"}"#.into(),
                PimdirActionError::UnknownVersion(None),
            ),
        ];
        for (kind, payload, expected) in cases {
            let err = action_from_payload(kind, &payload).unwrap_err();
            match (&err, &expected) {
                (
                    PimdirActionError::Invalid { field, .. },
                    PimdirActionError::Invalid { field: named, .. },
                ) => assert_eq!(field, named, "{kind} {payload}"),
                _ => assert_eq!(err, expected, "{kind} {payload}"),
            }
        }
    }

    #[test]
    fn an_application_kind_survives_whole_and_names_no_body() {
        let payload = r#"{"v":1,"object":5,"to":["+33"]}"#;
        let decoded = action_from_payload("x-fax-send", payload).unwrap();
        assert_eq!(decoded.kind(), "x-fax-send");
        assert_eq!(decoded.object_hash(), None, "the body is the row's");
        assert_eq!(action_to_payload(&decoded), payload);
    }
}
