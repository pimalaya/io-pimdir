//! # Encodings
//!
//! The I/O-free encodings between the model and the pimdir columns
//! (STORAGE §13), and the action queue's versioned payload (STORAGE
//! §15.3): the part an implementation holding its own SQLite binding
//! reuses.

use core::{fmt, str::FromStr};

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
    intent::{
        self, PimdirCollectionCreate, PimdirCollectionDelete, PimdirCollectionRename,
        PimdirInvitation, PimdirSubmit,
    },
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

/// The system flags a row holds as bits (STORAGE §13), each by its exact
/// bytes, in the order of their bits: `\Seen` 1, `\Answered` 2, `\Flagged`
/// 4, `\Draft` 8, `\Deleted` 16. Every other flag, `\seen` included, is a
/// row.
pub const MARKS: [&str; 5] = ["\\Seen", "\\Answered", "\\Flagged", "\\Draft", "\\Deleted"];

/// A flag's bit in `marks` or `base_marks` (STORAGE §13), `None` for a
/// flag held as a row.
pub fn mark_of(flag: &str) -> Option<i64> {
    MARKS
        .iter()
        .position(|mark| *mark == flag)
        .map(|position| 1 << position)
}

/// A set's system flags as the `marks` or `base_marks` column holds them
/// (STORAGE §13), `0` for an unknown set.
pub fn marks_to_int(flags: &PimdirFlags) -> i64 {
    flags
        .known()
        .into_iter()
        .flatten()
        .filter_map(|flag| mark_of(flag))
        .sum()
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

/// The identity of a store (STORAGE §4.5, §8): its id, drawn at its
/// creation, and its incarnation, raised when a copy of it is restored.
/// Another of either than the one recorded is a reset.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirStoreIdentity {
    /// The store's id, 32 lowercase hex digits.
    pub store_id: String,
    /// The store's incarnation, compared for equality alone.
    pub incarnation: i64,
}

/// The one reserved prefix of `queue.error` (STORAGE §13, §15.2): the
/// word, a colon and one space. A parked row whose error begins with it
/// is an intent whose outcome is unknown, neither performed nor failed,
/// and no other row's error begins so.
pub const UNCONFIRMED: &str = "unconfirmed: ";

/// The text a parked row's error states after the reserved prefix when
/// the row is an intent whose outcome is unknown (STORAGE §15.2, §15.4),
/// `None` for any other parked row.
pub fn unconfirmed(error: &str) -> Option<&str> {
    error.strip_prefix(UNCONFIRMED)
}

/// The handle of a queue row (STORAGE §4.5, §15.4): the id its enqueue
/// answered, under the identity of the store that drew it, which a
/// producer records with the id and checks when it follows or cancels
/// the row, since a restored copy may hand the id to another row.
///
/// It prints as the store id, the incarnation and the id joined by
/// periods (`e2646df031cd876334410c4df14ebd86.1.42`) and parses back. A
/// bare id (`42`, or an `i64`) is a handle stating no store: it is
/// followed unchecked, as before the identity was recorded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirActionHandle {
    /// The row's id in the queue.
    pub id: i64,
    /// The store the id was drawn in, `None` for a bare id.
    pub store: Option<PimdirStoreIdentity>,
}

impl PimdirActionHandle {
    /// Whether the handle may name a row of the store `identity` (§4.5):
    /// a bare id always, else only under the store id and the
    /// incarnation it was answered in. `false` is a reset: the id may
    /// name another row, or none, of a restored copy.
    pub fn names(&self, identity: &PimdirStoreIdentity) -> bool {
        self.store.as_ref().is_none_or(|store| store == identity)
    }
}

impl From<i64> for PimdirActionHandle {
    fn from(id: i64) -> Self {
        Self { id, store: None }
    }
}

impl From<&PimdirActionHandle> for PimdirActionHandle {
    fn from(handle: &PimdirActionHandle) -> Self {
        handle.clone()
    }
}

impl fmt::Display for PimdirActionHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.store {
            Some(store) => write!(f, "{}.{}.{}", store.store_id, store.incarnation, self.id),
            None => write!(f, "{}", self.id),
        }
    }
}

/// A string that is no queue row handle: neither an id nor a store id,
/// an incarnation and an id joined by periods.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PimdirActionHandleError;

impl fmt::Display for PimdirActionHandleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Pimdir action handle is neither an id nor a store id, an incarnation and an id joined by periods"
        )
    }
}

impl core::error::Error for PimdirActionHandleError {}

impl FromStr for PimdirActionHandle {
    type Err = PimdirActionHandleError;

    fn from_str(handle: &str) -> Result<Self, Self::Err> {
        let id = |id: &str| id.parse::<i64>().map_err(|_| PimdirActionHandleError);
        let parts: Vec<&str> = handle.split('.').collect();
        match parts[..] {
            [bare] => Ok(Self::from(id(bare)?)),
            [store_id, incarnation, row] if !store_id.is_empty() => Ok(Self {
                id: id(row)?,
                store: Some(PimdirStoreIdentity {
                    store_id: store_id.to_string(),
                    incarnation: id(incarnation)?,
                }),
            }),
            _ => Err(PimdirActionHandleError),
        }
    }
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
    /// Set the item's flags: replaced by `flags`, or, stating `base`,
    /// changed by what `flags` adds to and removes from it (§15.3).
    SetFlags {
        /// The item's public id.
        seq: i64,
        /// The new flag set.
        flags: PimdirFlags,
        /// The set the producer read and changed into `flags` (§15.3):
        /// the owner applies the change alone to the set the item holds
        /// then, so a flag a sync landed or removed since survives, and
        /// parks the row over an unknown set. `None` replaces the set,
        /// for a producer writing one it never read.
        base: Option<PimdirFlags>,
    },
    /// Remove the item from the collection; already absent is success.
    Remove {
        /// The item's public id.
        seq: i64,
        /// The body the producer read and deletes (§15.3): the owner
        /// parks the row when the item holds another; `None` removes
        /// whatever it holds.
        if_match: Option<PimdirHash>,
        /// The diverging bodies a person's resolution by deleting was
        /// decided over (§15.3), as an `update`'s: the remove settles
        /// the conflicts it meets only when it names each, and parks
        /// when it meets none. `None` for an ordinary remove, which
        /// parks over any conflict.
        resolves: Option<Vec<PimdirHash>>,
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
    /// The payload's leading `v` is another version than the one read,
    /// or, as `None`, missing or no integer.
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
            Self::UnknownVersion(None) => {
                write!(f, "Pimdir action payload states no integer version")
            }
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
        PimdirAction::SetFlags { seq, flags, base } => {
            map.insert("seq".into(), json!(seq));
            map.insert("flags".into(), flags_to_value(flags));
            if let Some(base) = base {
                map.insert("base".into(), flags_to_value(base));
            }
        }
        PimdirAction::Remove {
            seq,
            if_match,
            resolves,
        } => {
            map.insert("seq".into(), json!(seq));
            if let Some(if_match) = if_match {
                map.insert("if_match".into(), json!(if_match.0));
            }
            if let Some(resolves) = resolves {
                let names: Vec<&str> = resolves.iter().map(PimdirHash::as_str).collect();
                map.insert("resolves".into(), json!(names));
            }
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

/// The actions this crate holds a schema for at `v: 1` (STORAGE §15.3,
/// Annex B.2): the store's own and the intents.
const KNOWN_ACTIONS: [&str; 13] = [
    "add",
    "set-flags",
    "remove",
    "move",
    "copy",
    "update",
    "set-performer",
    intent::SUBMIT,
    intent::CALENDAR_REPLY,
    intent::CALENDAR_CANCEL,
    intent::COLLECTION_CREATE,
    intent::COLLECTION_RENAME,
    intent::COLLECTION_DELETE,
];

/// What a payload is to this crate (STORAGE §15.3): one of three, which
/// an owner applies, parks or leaves untouched, and a reader folds or
/// lists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PimdirPayloadOutcome {
    /// Its action and `v` name a schema, which accepts it.
    Valid,
    /// Its action and `v` name a schema, which refuses it, or its action
    /// is one this crate knows and it states no `v`, or one that is no
    /// integer: never enqueued by a producer, parked by an owner.
    Malformed,
    /// Its action, or the integer `v` it states for an action this crate
    /// knows, names no schema here, an application's own action
    /// included: read no further, left pending and untouched by an
    /// owner, listed by a reader, never parked.
    Unknown,
}

/// Judges a kind and its payload (STORAGE §15.3): unknown when the kind is
/// not one this crate holds a schema for, whatever the payload holds, or
/// when the payload of a known kind states an integer `v` other than 1;
/// malformed when a known kind's payload states no `v` or one that is no
/// integer, there being no version an owner could come to know; else
/// valid or malformed as [`action_from_payload`] reads it.
pub fn payload_outcome(kind: &str, payload: &str) -> PimdirPayloadOutcome {
    match unknown(kind, payload) {
        Ok(true) => PimdirPayloadOutcome::Unknown,
        Ok(false) if action_from_payload(kind, payload).is_ok() => PimdirPayloadOutcome::Valid,
        _ => PimdirPayloadOutcome::Malformed,
    }
}

/// Whether a payload is unknown to this crate (STORAGE §15.3): its kind
/// names no schema here, or its `v` is an integer other than 1. A payload
/// of a known kind that is no JSON object, states no `v` or states one
/// that is no integer (a string, a fraction, a boolean, `null`) is
/// malformed: it names no version a later owner would know.
///
/// An integer is one as JSON writes it (RFC 8259 §6), digits with
/// neither a fraction nor an exponent, which is what serde_json reads as
/// one: `1.0` and `1e0` are floats to it, so malformed, where a JSON
/// Schema validator takes both for the integer 1.
fn unknown(kind: &str, payload: &str) -> Result<bool, PimdirActionError> {
    if !KNOWN_ACTIONS.contains(&kind) {
        return Ok(true);
    }
    let value: Value = serde_json::from_str(payload).map_err(|_| PimdirActionError::Json)?;
    let Value::Object(map) = value else {
        return Err(PimdirActionError::Json);
    };

    match map.get("v") {
        Some(Value::Number(v)) if v.is_i64() || v.is_u64() => Ok(v.as_i64() != Some(1)),
        _ => Err(PimdirActionError::UnknownVersion(None)),
    }
}

/// Decodes a kind and its payload strictly, as payloads/ types it at
/// `v: 1` (STORAGE §15.3): a member missing, of another type, breaking its
/// rule or one the version does not define is an error the owner parks
/// with and a producer refuses at the enqueue.
///
/// A payload unknown to this crate ([`payload_outcome`]), a kind it holds
/// no schema for or a known one at another integer `v`, is no error: it
/// is read
/// no further and kept verbatim as [`PimdirAction::Unknown`], which an
/// owner skips and a reader lists. An intent of Annex B.2 is read whole
/// by its type in [`crate::intent`] and kept as [`PimdirAction::Unknown`]
/// too. Neither names its body here: that is the row's
/// `queue.object_hash`, which the queue reads fill in.
pub fn action_from_payload(kind: &str, payload: &str) -> Result<PimdirAction, PimdirActionError> {
    if unknown(kind, payload)? {
        return Ok(PimdirAction::Unknown {
            kind: kind.to_string(),
            payload: payload.to_string(),
            object_hash: None,
        });
    }

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
            let map = members(payload, &["seq", "flags", "base"])?;
            PimdirAction::SetFlags {
                seq: required(seq(map.get("seq"), "seq")?, "seq")?,
                flags: required(flags(map.get("flags"), "flags")?, "flags")?,
                base: flags(map.get("base"), "base")?,
            }
        }
        "remove" => {
            let map = members(payload, &["seq", "if_match", "resolves"])?;
            PimdirAction::Remove {
                seq: required(seq(map.get("seq"), "seq")?, "seq")?,
                if_match: hash(map.get("if_match"), "if_match")?,
                resolves: hashes(map.get("resolves"), "resolves")?,
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
                intent::COLLECTION_RENAME => {
                    PimdirCollectionRename::from_payload(payload)?;
                }
                _ => {
                    PimdirCollectionDelete::from_payload(payload)?;
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

        let mut removed = PimdirAction::Remove {
            seq: 3,
            if_match: None,
            resolves: None,
        };
        removed.retarget(to);
        assert_eq!(removed.target_collection(), None);
        assert_eq!(
            removed,
            PimdirAction::Remove {
                seq: 3,
                if_match: None,
                resolves: None
            }
        );
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
                base: None,
            },
            PimdirAction::Remove {
                seq: 5,
                if_match: None,
                resolves: None,
            },
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
            base: None,
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
                "set-flags",
                r#"{"v":1,"seq":1,"flags":[],"base":null}"#.into(),
                invalid("base"),
            ),
            (
                "remove",
                r#"{"v":1,"seq":1,"resolves":[]}"#.into(),
                invalid("resolves"),
            ),
            (
                "remove",
                r#"{"v":1,"seq":1,"if_match":"../x"}"#.into(),
                invalid("if_match"),
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

    /// A payload whose action, or whose `v` for a known action, names no
    /// schema here is unknown (STORAGE §15.3): kept verbatim and read no
    /// further, whatever its other members hold, never an error an owner
    /// would park with. Before draft-10's second batch a known action
    /// stating no `v` was refused as malformed, and one at `v` 2 was an
    /// error the callers had to tell apart.
    #[test]
    fn a_payload_this_crate_does_not_know_is_kept_never_refused() {
        let cases = [
            ("remove", r#"{"v":2,"seq":1}"#),
            ("remove", r#"{"v":18446744073709551615,"seq":1}"#),
            ("set-flags", r#"{"v":2,"seq":"5"}"#),
            ("submit", r#"{"v":2}"#),
            ("snooze", r#"{"v":1,"seq":5}"#),
            ("snooze", r#"{"seq":5}"#),
            ("x-fax-send", r#"{"to":"+33"}"#),
            ("x-fax-send", r#"{"v":"one"}"#),
            ("x-fax-send", "not json"),
        ];
        for (kind, payload) in cases {
            assert_eq!(
                payload_outcome(kind, payload),
                PimdirPayloadOutcome::Unknown,
                "{kind} {payload}"
            );
            let kept = PimdirAction::Unknown {
                kind: kind.into(),
                payload: payload.into(),
                object_hash: None,
            };
            assert_eq!(action_from_payload(kind, payload), Ok(kept));
        }

        assert_eq!(
            payload_outcome("remove", r#"{"v":1,"seq":1}"#),
            PimdirPayloadOutcome::Valid
        );
        assert_eq!(
            payload_outcome("remove", r#"{"v":1,"seq":0}"#),
            PimdirPayloadOutcome::Malformed
        );
        assert_eq!(
            payload_outcome("remove", "not json"),
            PimdirPayloadOutcome::Malformed,
            "a known action whose payload states no v to read"
        );
    }

    /// A known action stating no `v`, or one that is no integer, states
    /// no version an owner could wait to know: malformed, never unknown
    /// (STORAGE §15.3). An integer is written with neither a fraction nor
    /// an exponent, so `1.0` and `1e0` are none.
    #[test]
    fn a_known_action_stating_no_usable_version_is_malformed() {
        let cases = [
            ("remove", r#"{"seq":1}"#),
            ("remove", r#"{"v":"1","seq":1}"#),
            ("remove", r#"{"v":1.5,"seq":1}"#),
            ("remove", r#"{"v":1.0,"seq":1}"#),
            ("remove", r#"{"v":1e0,"seq":1}"#),
            ("remove", r#"{"v":1E+0,"seq":1}"#),
            ("remove", r#"{"v":null,"seq":1}"#),
            ("remove", r#"{"v":true,"seq":1}"#),
            ("submit", r#"{"v":"2"}"#),
        ];
        for (kind, payload) in cases {
            assert_eq!(
                payload_outcome(kind, payload),
                PimdirPayloadOutcome::Malformed,
                "{kind} {payload}"
            );
            assert_eq!(
                action_from_payload(kind, payload),
                Err(PimdirActionError::UnknownVersion(None)),
                "{kind} {payload}"
            );
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
