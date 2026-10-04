//! # Intents
//!
//! The typed payloads of the Annex B.2 intents this crate validates: a
//! collection created on a source's server, and the invitation reply and
//! cancel with their optional occurrence. An intent travels as
//! [`PimdirAction::Unknown`], so the store keeps its payload verbatim and
//! skips it; these types build that action for a producer and read it
//! back for a performer, refusing a malformed payload with the error the
//! owner parks the row with. I/O-free.

use alloc::string::{String, ToString};

use serde_json::{Map, Value, json};

use crate::{
    codec::{PimdirAction, PimdirActionError},
    collection::PimdirCollectionId,
    placement::PimdirLinkId,
};

/// The kind of the `mail.submit` intent.
pub const SUBMIT: &str = "submit";
/// The kind of the `calendar.reply` intent.
pub const CALENDAR_REPLY: &str = "calendar-reply";
/// The kind of the `calendar.cancel` intent.
pub const CALENDAR_CANCEL: &str = "calendar-cancel";
/// The kind of the `collection.create` intent.
pub const COLLECTION_CREATE: &str = "collection-create";

/// A collection to create on the performer's server (Annex B.2,
/// `collection-create`).
///
/// Anchored on `parent` when it names one, else on a collection of the
/// account and the kind the new one takes ([`anchor`](Self::anchor)).
/// The collection reaches the store with the performer's next sync,
/// labelled `name`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirCollectionCreate {
    /// The source performing it, `None` only from a producer predating
    /// §15.6.
    pub source: Option<String>,
    /// One level of the hierarchy: a non-empty label, mapped by the
    /// performer onto its server.
    pub name: String,
    /// The collection to create it under, `None` for the top level.
    pub parent: Option<PimdirCollectionId>,
}

impl PimdirCollectionCreate {
    /// The collection the intent is anchored on (§15.6): `parent` when
    /// named, else `fallback`, a collection of the account and kind the
    /// new one takes.
    pub fn anchor<'a>(&'a self, fallback: &'a str) -> &'a str {
        match &self.parent {
            Some(parent) => parent.as_str(),
            None => fallback,
        }
    }

    /// Checks the payload's own rules: a `name` neither blank nor holding
    /// a control character.
    pub fn validate(&self) -> Result<(), PimdirActionError> {
        if self.name.trim().is_empty() {
            return Err(PimdirActionError::Invalid {
                field: "name",
                reason: "a collection-create needs a non-blank name",
            });
        }
        if self.name.chars().any(char::is_control) {
            return Err(PimdirActionError::Invalid {
                field: "name",
                reason: "a collection name holds no control character",
            });
        }
        if self.parent.as_ref().is_some_and(|p| p.as_str().is_empty()) {
            return Err(PimdirActionError::Invalid {
                field: "parent",
                reason: "a parent names a collection",
            });
        }
        Ok(())
    }

    /// The queue action carrying it, validated first.
    pub fn to_action(&self) -> Result<PimdirAction, PimdirActionError> {
        self.validate()?;
        let mut map = versioned();
        if let Some(source) = &self.source {
            map.insert("source".into(), json!(source));
        }
        map.insert("name".into(), json!(self.name));
        if let Some(parent) = &self.parent {
            map.insert("parent".into(), json!(parent.0));
        }
        Ok(intent(COLLECTION_CREATE, map))
    }

    /// Reads a `collection-create` payload strictly.
    pub fn from_payload(payload: &str) -> Result<Self, PimdirActionError> {
        let map = object(payload)?;
        let create = Self {
            source: string(&map, "source")?,
            name: string(&map, "name")?.ok_or(PimdirActionError::MissingField("name"))?,
            parent: string(&map, "parent")?.map(PimdirCollectionId),
        };
        create.validate()?;
        Ok(create)
    }
}

/// The item an invitation intent addresses: exactly one of the two
/// (Annex B.2).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirIntentItem {
    /// An item of the store, by its public id.
    Seq(i64),
    /// An item still a pending `add` of the producer, by its key.
    Link(PimdirLinkId),
}

/// The participation a reply sends (RFC 5546 §3.2.3).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PimdirPartstat {
    /// `ACCEPTED`.
    Accepted,
    /// `TENTATIVE`.
    Tentative,
    /// `DECLINED`.
    Declined,
}

impl PimdirPartstat {
    /// The value as the payload spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "ACCEPTED",
            Self::Tentative => "TENTATIVE",
            Self::Declined => "DECLINED",
        }
    }

    /// The inverse of [`as_str`](Self::as_str); anything else is `None`.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ACCEPTED" => Some(Self::Accepted),
            "TENTATIVE" => Some(Self::Tentative),
            "DECLINED" => Some(Self::Declined),
            _ => None,
        }
    }
}

/// An invitation reply or cancel (Annex B.2, `calendar-reply` and
/// `calendar-cancel`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirInvitation {
    /// The source performing it, `None` only from a producer predating
    /// §15.6.
    pub source: Option<String>,
    /// The item it addresses.
    pub item: PimdirIntentItem,
    /// The reply's participation, `None` for a cancel.
    pub partstat: Option<PimdirPartstat>,
    /// The comment sent with it.
    pub comment: Option<String>,
    /// The occurrence it is limited to, by its `RECURRENCE-ID` value as
    /// the item spells it; `None` for the whole series.
    pub recurrence_id: Option<String>,
}

impl PimdirInvitation {
    /// The intent kind: a reply when it carries a participation.
    pub fn kind(&self) -> &'static str {
        match self.partstat {
            Some(_) => CALENDAR_REPLY,
            None => CALENDAR_CANCEL,
        }
    }

    /// The queue action carrying it, validated first.
    pub fn to_action(&self) -> Result<PimdirAction, PimdirActionError> {
        if let Some(recurrence_id) = &self.recurrence_id {
            validate_recurrence_id(recurrence_id)?;
        }
        let mut map = versioned();
        if let Some(source) = &self.source {
            map.insert("source".into(), json!(source));
        }
        match &self.item {
            PimdirIntentItem::Seq(seq) => map.insert("seq".into(), json!(seq)),
            PimdirIntentItem::Link(link) => map.insert("link_id".into(), json!(link.0)),
        };
        if let Some(partstat) = self.partstat {
            map.insert("partstat".into(), json!(partstat.as_str()));
        }
        if let Some(comment) = &self.comment {
            map.insert("comment".into(), json!(comment));
        }
        if let Some(recurrence_id) = &self.recurrence_id {
            map.insert("recurrence_id".into(), json!(recurrence_id));
        }
        Ok(intent(self.kind(), map))
    }

    /// Reads a `calendar-reply` or `calendar-cancel` payload strictly.
    pub fn from_payload(kind: &str, payload: &str) -> Result<Self, PimdirActionError> {
        let map = object(payload)?;
        let item = match (map.get("seq"), string(&map, "link_id")?) {
            (Some(seq), None) => {
                PimdirIntentItem::Seq(seq.as_i64().ok_or(PimdirActionError::MissingField("seq"))?)
            }
            (None, Some(link)) => PimdirIntentItem::Link(PimdirLinkId(link)),
            (None, None) => return Err(PimdirActionError::MissingField("seq")),
            (Some(_), Some(_)) => {
                return Err(PimdirActionError::Invalid {
                    field: "seq",
                    reason: "an intent names its item by exactly one of seq and link_id",
                });
            }
        };
        let partstat = match kind {
            CALENDAR_REPLY => Some(
                string(&map, "partstat")?
                    .as_deref()
                    .and_then(PimdirPartstat::parse)
                    .ok_or(PimdirActionError::MissingField("partstat"))?,
            ),
            _ => None,
        };
        let recurrence_id = string(&map, "recurrence_id")?;
        if let Some(recurrence_id) = &recurrence_id {
            validate_recurrence_id(recurrence_id)?;
        }
        Ok(Self {
            source: string(&map, "source")?,
            item,
            partstat,
            comment: string(&map, "comment")?,
            recurrence_id,
        })
    }
}

/// The occurrence an invitation payload names, `Ok(None)` when it names
/// none, or the error when the field is there and malformed: what the
/// gate reads without decoding the rest of the payload.
pub fn recurrence_id(payload: &str) -> Result<Option<String>, PimdirActionError> {
    let map = object(payload)?;
    let recurrence_id = string(&map, "recurrence_id")?;
    if let Some(recurrence_id) = &recurrence_id {
        validate_recurrence_id(recurrence_id)?;
    }
    Ok(recurrence_id)
}

/// Checks a `RECURRENCE-ID` value as Annex B.2 spells it: a date
/// `YYYYMMDD`, or a date-time `YYYYMMDDTHHMMSS` with an optional `Z`
/// (RFC 5545 §3.3.4, §3.3.5), parameters aside.
pub fn validate_recurrence_id(value: &str) -> Result<(), PimdirActionError> {
    let bytes = value.as_bytes();
    let digits = |range: core::ops::Range<usize>| bytes[range].iter().all(u8::is_ascii_digit);
    let valid = match bytes.len() {
        8 => digits(0..8),
        15 | 16 => {
            digits(0..8)
                && bytes[8] == b'T'
                && digits(9..15)
                && (bytes.len() == 15 || bytes[15] == b'Z')
        }
        _ => false,
    };
    match valid {
        true => Ok(()),
        false => Err(PimdirActionError::Invalid {
            field: "recurrence_id",
            reason: "a RECURRENCE-ID value is YYYYMMDD or YYYYMMDDTHHMMSS[Z]",
        }),
    }
}

fn versioned() -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("v".into(), json!(1));
    map
}

fn intent(kind: &str, map: Map<String, Value>) -> PimdirAction {
    PimdirAction::Unknown {
        kind: kind.to_string(),
        payload: Value::Object(map).to_string(),
        object_hash: None,
    }
}

/// The payload's object at `v: 1`.
fn object(payload: &str) -> Result<Map<String, Value>, PimdirActionError> {
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

fn string(
    map: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, PimdirActionError> {
    match map.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(PimdirActionError::MissingField(field)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec;

    #[test]
    fn a_collection_create_round_trips_through_the_queue_codec() {
        let create = PimdirCollectionCreate {
            source: Some("imap".into()),
            name: "Archives".into(),
            parent: Some(PimdirCollectionId::from("INBOX")),
        };
        let action = create.to_action().unwrap();
        assert_eq!(action.kind(), COLLECTION_CREATE);
        assert_eq!(action.object_hash(), None);

        let payload = codec::action_to_payload(&action);
        assert_eq!(
            payload,
            r#"{"name":"Archives","parent":"INBOX","source":"imap","v":1}"#
        );
        let decoded = codec::action_from_payload(COLLECTION_CREATE, &payload).unwrap();
        assert_eq!(decoded, action);
        assert_eq!(
            PimdirCollectionCreate::from_payload(&payload).unwrap(),
            create
        );
        assert_eq!(create.anchor("Sent"), "INBOX");

        let top = PimdirCollectionCreate {
            parent: None,
            ..create
        };
        assert_eq!(top.anchor("Sent"), "Sent");
        assert!(!codec::action_to_payload(&top.to_action().unwrap()).contains("parent"));
    }

    #[test]
    fn a_collection_create_needs_a_name() {
        let refused = |payload: &str| PimdirCollectionCreate::from_payload(payload).unwrap_err();

        assert_eq!(
            refused(r#"{"v":1,"source":"imap"}"#),
            PimdirActionError::MissingField("name")
        );
        assert!(matches!(
            refused(r#"{"v":1,"name":"  "}"#),
            PimdirActionError::Invalid { field: "name", .. }
        ));
        assert!(matches!(
            refused("{\"v\":1,\"name\":\"a\\nb\"}"),
            PimdirActionError::Invalid { field: "name", .. }
        ));
        assert_eq!(
            refused(r#"{"v":1,"name":3}"#),
            PimdirActionError::MissingField("name")
        );
        assert_eq!(
            refused(r#"{"v":2,"name":"a"}"#),
            PimdirActionError::UnknownVersion(Some(2))
        );
        assert!(matches!(
            refused(r#"{"v":1,"name":"a","parent":""}"#),
            PimdirActionError::Invalid {
                field: "parent",
                ..
            }
        ));
    }

    #[test]
    fn an_invitation_names_its_occurrence_or_the_whole_series() {
        let reply = PimdirInvitation {
            source: Some("dav".into()),
            item: PimdirIntentItem::Seq(4),
            partstat: Some(PimdirPartstat::Declined),
            comment: None,
            recurrence_id: Some("20261006T070000Z".into()),
        };
        let action = reply.to_action().unwrap();
        assert_eq!(action.kind(), CALENDAR_REPLY);
        let PimdirAction::Unknown { payload, .. } = &action else {
            panic!("an intent travels as an unknown kind");
        };
        assert_eq!(
            PimdirInvitation::from_payload(CALENDAR_REPLY, payload).unwrap(),
            reply
        );
        assert_eq!(
            recurrence_id(payload).unwrap().as_deref(),
            Some("20261006T070000Z")
        );

        // NOTE: a payload from before the field addresses the series.
        let series = r#"{"v":1,"source":"dav","seq":4,"comment":"sorry"}"#;
        let cancel = PimdirInvitation::from_payload(CALENDAR_CANCEL, series).unwrap();
        assert_eq!(cancel.recurrence_id, None);
        assert_eq!(cancel.partstat, None);
        assert_eq!(cancel.kind(), CALENDAR_CANCEL);
        assert_eq!(recurrence_id(series).unwrap(), None);
    }

    #[test]
    fn an_invitation_refuses_a_malformed_occurrence_or_item() {
        for value in ["2026-10-06", "20261006T0700", "20261006T070000+0200", ""] {
            let payload = alloc::format!(r#"{{"v":1,"seq":1,"recurrence_id":"{value}"}}"#);
            assert!(
                matches!(
                    recurrence_id(&payload),
                    Err(PimdirActionError::Invalid {
                        field: "recurrence_id",
                        ..
                    })
                ),
                "{value} refused"
            );
        }
        for value in ["20261006", "20261006T090000", "20261006T070000Z"] {
            assert_eq!(validate_recurrence_id(value), Ok(()), "{value} accepted");
        }

        let both = r#"{"v":1,"seq":1,"link_id":"uid:a","partstat":"ACCEPTED"}"#;
        assert!(matches!(
            PimdirInvitation::from_payload(CALENDAR_REPLY, both),
            Err(PimdirActionError::Invalid { field: "seq", .. })
        ));
        let pending = r#"{"v":1,"link_id":"uid:a","partstat":"MAYBE"}"#;
        assert_eq!(
            PimdirInvitation::from_payload(CALENDAR_REPLY, pending),
            Err(PimdirActionError::MissingField("partstat"))
        );
        let pending = r#"{"v":1,"link_id":"uid:a"}"#;
        assert_eq!(
            PimdirInvitation::from_payload(CALENDAR_CANCEL, pending)
                .unwrap()
                .item,
            PimdirIntentItem::Link(PimdirLinkId::from("uid:a"))
        );
    }
}
