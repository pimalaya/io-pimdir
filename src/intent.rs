//! # Intents
//!
//! The typed payloads of the Annex B.2 intents this crate validates: a
//! message to send, a collection created, renamed or deleted on a
//! source's server, and the invitation reply and cancel with their
//! optional occurrence. An intent
//! travels as [`PimdirAction::Unknown`], so the store keeps its payload
//! verbatim and skips it; these types build that action for a producer
//! and read it back for a performer, strictly as payloads/ types it
//! (STORAGE §15.3), refusing a malformed payload with the error the owner
//! parks the row with. I/O-free.

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use serde_json::{Map, Value, json};

use crate::{
    codec::{self, PimdirAction, PimdirActionError},
    collection::PimdirCollectionId,
    object::PimdirHash,
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
/// The kind of the `collection.rename` intent.
pub const COLLECTION_RENAME: &str = "collection-rename";
/// The kind of the `collection.delete` intent.
pub const COLLECTION_DELETE: &str = "collection-delete";

/// A collection to create on the performer's server (Annex B.2,
/// `collection-create`).
///
/// Anchored on `parent` when it names one, else on a collection of the
/// account and the kind the new one takes ([`anchor`](Self::anchor)).
/// The collection reaches the store with the performer's next sync,
/// labelled `name`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirCollectionCreate {
    /// The source performing it (§15.6).
    pub source: String,
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

    /// Checks the payload's own rules: a named `source`, a `name` neither
    /// blank nor holding a control character, a `parent` naming a
    /// collection.
    pub fn validate(&self) -> Result<(), PimdirActionError> {
        validate_source(&self.source)?;
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
        map.insert("source".into(), json!(self.source));
        map.insert("name".into(), json!(self.name));
        if let Some(parent) = &self.parent {
            map.insert("parent".into(), json!(parent.0));
        }
        Ok(intent(COLLECTION_CREATE, map))
    }

    /// Reads a `collection-create` payload strictly.
    pub fn from_payload(payload: &str) -> Result<Self, PimdirActionError> {
        let map = codec::members(payload, &["source", "name", "parent"])?;
        let create = Self {
            source: codec::required(codec::label(map.get("source"), "source")?, "source")?,
            name: codec::required(codec::label(map.get("name"), "name")?, "name")?,
            parent: codec::label(map.get("parent"), "parent")?.map(PimdirCollectionId),
        };
        create.validate()?;
        Ok(create)
    }
}

/// The collection the action is queued on, renamed on the performer's
/// server (Annex B.2, `collection-rename`).
///
/// Anchored on that collection, which the payload does not restate, so a
/// rename of it in the store leaves the row addressing it. The parent and
/// the items are kept, and the new name reaches the store with the
/// performer's next sync.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirCollectionRename {
    /// The source performing it (§15.6).
    pub source: String,
    /// The new label, one level of the hierarchy, as
    /// [`PimdirCollectionCreate::name`] is.
    pub name: String,
}

impl PimdirCollectionRename {
    /// Checks the payload's own rules: a named `source`, a `name` neither
    /// blank nor holding a control character.
    pub fn validate(&self) -> Result<(), PimdirActionError> {
        validate_source(&self.source)?;
        if self.name.trim().is_empty() {
            return Err(PimdirActionError::Invalid {
                field: "name",
                reason: "a collection-rename needs a non-blank name",
            });
        }
        if self.name.chars().any(char::is_control) {
            return Err(PimdirActionError::Invalid {
                field: "name",
                reason: "a collection name holds no control character",
            });
        }
        Ok(())
    }

    /// The queue action carrying it, validated first.
    pub fn to_action(&self) -> Result<PimdirAction, PimdirActionError> {
        self.validate()?;
        let mut map = versioned();
        map.insert("source".into(), json!(self.source));
        map.insert("name".into(), json!(self.name));
        Ok(intent(COLLECTION_RENAME, map))
    }

    /// Reads a `collection-rename` payload strictly.
    pub fn from_payload(payload: &str) -> Result<Self, PimdirActionError> {
        let map = codec::members(payload, &["source", "name"])?;
        let rename = Self {
            source: codec::required(codec::label(map.get("source"), "source")?, "source")?,
            name: codec::required(codec::label(map.get("name"), "name")?, "name")?,
        };
        rename.validate()?;
        Ok(rename)
    }
}

/// The collection the action is queued on, deleted on the performer's
/// server with the items it holds (Annex B.2, `collection-delete`).
///
/// Anchored on that collection, never taking another: a performer
/// finding a collection under it on the server parks the row. The store
/// removes no item for it, the deletion reaching it with the performer's
/// next sync.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirCollectionDelete {
    /// The source performing it (§15.6).
    pub source: String,
}

impl PimdirCollectionDelete {
    /// Checks the payload's own rule: a named `source`.
    pub fn validate(&self) -> Result<(), PimdirActionError> {
        validate_source(&self.source)
    }

    /// The queue action carrying it, validated first.
    pub fn to_action(&self) -> Result<PimdirAction, PimdirActionError> {
        self.validate()?;
        let mut map = versioned();
        map.insert("source".into(), json!(self.source));
        Ok(intent(COLLECTION_DELETE, map))
    }

    /// Reads a `collection-delete` payload strictly.
    pub fn from_payload(payload: &str) -> Result<Self, PimdirActionError> {
        let map = codec::members(payload, &["source"])?;
        let delete = Self {
            source: codec::required(codec::label(map.get("source"), "source")?, "source")?,
        };
        delete.validate()?;
        Ok(delete)
    }
}

/// A message to send (Annex B.2, `submit`, capability `mail.submit`).
///
/// The message is the action's body, the row's `queue.object_hash`, never
/// a member of the payload, which carries the envelope it is sent with,
/// its `Bcc` field removed before transmission. Anchored on the
/// collection the sent copy belongs to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirSubmit {
    /// The source performing it (§15.6).
    pub source: String,
    /// The envelope sender (`MAIL FROM`).
    pub from: String,
    /// The envelope recipients (`RCPT TO`), at least one.
    pub rcpts: Vec<String>,
    /// The subject, for reports only.
    pub subject: Option<String>,
    /// The collection a copy is filed in once sent (`mail.submit.copy`),
    /// `None` for no copy.
    pub copy: Option<PimdirCollectionId>,
}

impl PimdirSubmit {
    /// Checks the payload's own rules: a named `source` and `from`, at
    /// least one recipient, none blank, and a `copy` naming a collection.
    pub fn validate(&self) -> Result<(), PimdirActionError> {
        validate_source(&self.source)?;
        if self.from.is_empty() {
            return Err(PimdirActionError::Invalid {
                field: "from",
                reason: "a submit names its envelope sender",
            });
        }
        if self.rcpts.is_empty() {
            return Err(PimdirActionError::Invalid {
                field: "rcpts",
                reason: "a submit names at least one recipient",
            });
        }
        if self.rcpts.iter().any(|rcpt| rcpt.trim().is_empty()) {
            return Err(PimdirActionError::Invalid {
                field: "rcpts",
                reason: "a recipient is an address",
            });
        }
        if self.copy.as_ref().is_some_and(|c| c.as_str().is_empty()) {
            return Err(PimdirActionError::Invalid {
                field: "copy",
                reason: "a copy names a collection",
            });
        }
        Ok(())
    }

    /// The queue action sending `message`, validated first: the message
    /// is the action's body, its `object_hash`, which the enqueue pins.
    pub fn to_action(&self, message: PimdirHash) -> Result<PimdirAction, PimdirActionError> {
        self.validate()?;
        let mut map = versioned();
        map.insert("source".into(), json!(self.source));
        map.insert("from".into(), json!(self.from));
        map.insert("rcpts".into(), json!(self.rcpts));
        if let Some(subject) = &self.subject {
            map.insert("subject".into(), json!(subject));
        }
        if let Some(copy) = &self.copy {
            map.insert("copy".into(), json!(copy.0));
        }
        Ok(PimdirAction::Unknown {
            kind: SUBMIT.to_string(),
            payload: Value::Object(map).to_string(),
            object_hash: Some(message),
        })
    }

    /// Reads a `submit` payload strictly.
    pub fn from_payload(payload: &str) -> Result<Self, PimdirActionError> {
        let map = codec::members(payload, &["source", "from", "rcpts", "subject", "copy"])?;
        let rcpts = match map.get("rcpts") {
            Some(Value::Array(rcpts)) => rcpts
                .iter()
                .map(|rcpt| codec::required(codec::label(Some(rcpt), "rcpts")?, "rcpts"))
                .collect::<Result<Vec<_>, _>>()?,
            Some(_) => {
                return Err(PimdirActionError::Invalid {
                    field: "rcpts",
                    reason: "an array of addresses is expected",
                });
            }
            None => return Err(PimdirActionError::MissingField("rcpts")),
        };
        let submit = Self {
            source: codec::required(codec::label(map.get("source"), "source")?, "source")?,
            from: codec::required(codec::label(map.get("from"), "from")?, "from")?,
            rcpts,
            subject: codec::text(map.get("subject"), "subject")?,
            copy: codec::label(map.get("copy"), "copy")?.map(PimdirCollectionId),
        };
        submit.validate()?;
        Ok(submit)
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
    /// The source performing it (§15.6).
    pub source: String,
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

    /// Checks the payload's own rules: a named `source`, an item named by
    /// a `seq` from 1 or a non-empty `link_id`, and a well-formed
    /// `recurrence_id`.
    pub fn validate(&self) -> Result<(), PimdirActionError> {
        validate_source(&self.source)?;
        match &self.item {
            PimdirIntentItem::Seq(seq) if *seq < 1 => {
                return Err(PimdirActionError::Invalid {
                    field: "seq",
                    reason: "an integer from 1 is expected",
                });
            }
            PimdirIntentItem::Link(link) if link.0.is_empty() => {
                return Err(PimdirActionError::Invalid {
                    field: "link_id",
                    reason: "a non-empty string is expected",
                });
            }
            _ => {}
        }
        if let Some(recurrence_id) = &self.recurrence_id {
            validate_recurrence_id(recurrence_id)?;
        }
        Ok(())
    }

    /// The queue action carrying it, validated first.
    pub fn to_action(&self) -> Result<PimdirAction, PimdirActionError> {
        self.validate()?;
        let mut map = versioned();
        map.insert("source".into(), json!(self.source));
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

    /// Reads a `calendar-reply` payload strictly when `kind` names it,
    /// else a `calendar-cancel` one, which states no `partstat`.
    pub fn from_payload(kind: &str, payload: &str) -> Result<Self, PimdirActionError> {
        let map = match kind {
            CALENDAR_REPLY => codec::members(payload, &INVITATION)?,
            _ => codec::members(payload, &INVITATION[..INVITATION.len() - 1])?,
        };
        let seq = codec::seq(map.get("seq"), "seq")?;
        let link = codec::label(map.get("link_id"), "link_id")?;
        let item = match (seq, link) {
            (Some(seq), None) => PimdirIntentItem::Seq(seq),
            (None, Some(link)) => PimdirIntentItem::Link(PimdirLinkId(link)),
            _ => {
                return Err(PimdirActionError::Invalid {
                    field: "seq",
                    reason: "an intent names its item by exactly one of seq and link_id",
                });
            }
        };
        let partstat = match kind {
            CALENDAR_REPLY => {
                let partstat =
                    codec::required(codec::text(map.get("partstat"), "partstat")?, "partstat")?;
                let partstat =
                    PimdirPartstat::parse(&partstat).ok_or(PimdirActionError::Invalid {
                        field: "partstat",
                        reason: "ACCEPTED, TENTATIVE or DECLINED is expected",
                    })?;
                Some(partstat)
            }
            _ => None,
        };
        let invitation = Self {
            source: codec::required(codec::label(map.get("source"), "source")?, "source")?,
            item,
            partstat,
            comment: codec::text(map.get("comment"), "comment")?,
            recurrence_id: codec::text(map.get("recurrence_id"), "recurrence_id")?,
        };
        invitation.validate()?;
        Ok(invitation)
    }
}

/// The members an invitation payload defines at `v: 1`, a reply's
/// `partstat` last.
const INVITATION: [&str; 6] = [
    "source",
    "seq",
    "link_id",
    "comment",
    "recurrence_id",
    "partstat",
];

/// The occurrence an invitation payload names, `Ok(None)` when it names
/// none, or the error when the field is there and malformed: what the
/// gate reads without decoding the rest of the payload.
pub fn recurrence_id(payload: &str) -> Result<Option<String>, PimdirActionError> {
    let map = codec::members(payload, &INVITATION)?;
    let recurrence_id = codec::text(map.get("recurrence_id"), "recurrence_id")?;
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

/// Checks an intent names the source performing it (§15.6).
fn validate_source(source: &str) -> Result<(), PimdirActionError> {
    match source.is_empty() {
        true => Err(PimdirActionError::Invalid {
            field: "source",
            reason: "an intent names the source performing it",
        }),
        false => Ok(()),
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

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;
    use crate::codec;

    /// The object name of the empty body (objects.json).
    const EMPTY: &str = "v4jutopv7gq2nicajxvdnxgjjgn4wjojvxarfn6mtkj4vza7gjra";

    #[test]
    fn a_submit_round_trips_through_the_queue_codec() {
        let submit = PimdirSubmit {
            source: "smtp".into(),
            from: "a@x.org".into(),
            rcpts: vec!["b@y.org".into(), "c@y.org".into()],
            subject: Some("hi".into()),
            copy: Some(PimdirCollectionId::from("Sent")),
        };
        let message = PimdirHash(EMPTY.into());
        let action = submit.to_action(message.clone()).unwrap();
        assert_eq!(action.kind(), SUBMIT);
        assert_eq!(action.object_hash(), Some(&message));

        let payload = codec::action_to_payload(&action);
        assert!(!payload.contains("object"), "the body is the row's");
        let decoded = codec::action_from_payload(SUBMIT, &payload).unwrap();
        assert_eq!(decoded.kind(), SUBMIT);
        assert_eq!(codec::action_to_payload(&decoded), payload);
        assert_eq!(PimdirSubmit::from_payload(&payload).unwrap(), submit);
    }

    /// The payload a frontend hand-rolled before the schemas: its body
    /// named in the payload, no source, a `null` subject.
    #[test]
    fn a_submit_refuses_the_payload_frontends_wrote() {
        let payload =
            alloc::format!(r#"{{"v":1,"object":"{EMPTY}","from":"a@x.org","rcpts":["b@y.org"]}}"#);
        assert_eq!(
            PimdirSubmit::from_payload(&payload),
            Err(PimdirActionError::UnknownField("object".into()))
        );
        let payload = r#"{"v":1,"from":"a@x.org","rcpts":["b@y.org"]}"#;
        assert_eq!(
            PimdirSubmit::from_payload(payload),
            Err(PimdirActionError::MissingField("source"))
        );
        let payload =
            r#"{"v":1,"source":"smtp","from":"a@x.org","rcpts":["b@y.org"],"subject":null}"#;
        assert!(matches!(
            PimdirSubmit::from_payload(payload),
            Err(PimdirActionError::Invalid {
                field: "subject",
                ..
            })
        ));
    }

    #[test]
    fn a_submit_needs_a_sender_and_a_recipient() {
        let refused = |envelope: &str| {
            let payload = alloc::format!(r#"{{"v":1,"source":"smtp",{envelope}}}"#);
            PimdirSubmit::from_payload(&payload).unwrap_err()
        };

        assert_eq!(
            refused(r#""rcpts":["b@y.org"]"#),
            PimdirActionError::MissingField("from")
        );
        assert!(matches!(
            refused(r#""from":"","rcpts":["b@y.org"]"#),
            PimdirActionError::Invalid { field: "from", .. }
        ));
        assert_eq!(
            refused(r#""from":"a@x.org""#),
            PimdirActionError::MissingField("rcpts")
        );
        for rcpts in ["[1]", "\"b@y.org\"", "[]", "[\" \"]"] {
            let err = refused(&alloc::format!(r#""from":"a@x.org","rcpts":{rcpts}"#));
            assert!(
                matches!(err, PimdirActionError::Invalid { field: "rcpts", .. }),
                "{rcpts}: {err:?}"
            );
        }
        assert_eq!(
            PimdirSubmit::from_payload(r#"{"v":2,"source":"smtp"}"#),
            Err(PimdirActionError::UnknownVersion(Some(2)))
        );
    }

    #[test]
    fn a_collection_create_round_trips_through_the_queue_codec() {
        let create = PimdirCollectionCreate {
            source: "imap".into(),
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
    fn a_collection_create_needs_a_source_and_a_name() {
        let refused = |payload: &str| PimdirCollectionCreate::from_payload(payload).unwrap_err();

        assert_eq!(
            refused(r#"{"v":1,"source":"imap"}"#),
            PimdirActionError::MissingField("name")
        );
        assert_eq!(
            refused(r#"{"v":1,"name":"a"}"#),
            PimdirActionError::MissingField("source")
        );
        assert!(matches!(
            refused(r#"{"v":1,"source":"imap","name":"  "}"#),
            PimdirActionError::Invalid { field: "name", .. }
        ));
        assert!(matches!(
            refused("{\"v\":1,\"source\":\"imap\",\"name\":\"a\\nb\"}"),
            PimdirActionError::Invalid { field: "name", .. }
        ));
        assert!(matches!(
            refused(r#"{"v":1,"source":"imap","name":3}"#),
            PimdirActionError::Invalid { field: "name", .. }
        ));
        assert_eq!(
            refused(r#"{"v":2,"name":"a"}"#),
            PimdirActionError::UnknownVersion(Some(2))
        );
        assert!(matches!(
            refused(r#"{"v":1,"source":"imap","name":"a","parent":""}"#),
            PimdirActionError::Invalid {
                field: "parent",
                ..
            }
        ));
        assert_eq!(
            refused(r#"{"v":1,"source":"imap","name":"a","seq":1}"#),
            PimdirActionError::UnknownField("seq".into())
        );
    }

    #[test]
    fn an_invitation_names_its_occurrence_or_the_whole_series() {
        let reply = PimdirInvitation {
            source: "dav".into(),
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
            let payload =
                alloc::format!(r#"{{"v":1,"source":"dav","seq":1,"recurrence_id":"{value}"}}"#);
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

        let refused =
            |kind: &str, payload: &str| PimdirInvitation::from_payload(kind, payload).unwrap_err();
        for payload in [
            r#"{"v":1,"source":"dav","seq":1,"link_id":"uid:a","partstat":"ACCEPTED"}"#,
            r#"{"v":1,"source":"dav","partstat":"ACCEPTED"}"#,
        ] {
            assert!(matches!(
                refused(CALENDAR_REPLY, payload),
                PimdirActionError::Invalid { field: "seq", .. }
            ));
        }
        assert!(matches!(
            refused(
                CALENDAR_REPLY,
                r#"{"v":1,"source":"dav","link_id":"uid:a","partstat":"MAYBE"}"#
            ),
            PimdirActionError::Invalid {
                field: "partstat",
                ..
            }
        ));
        assert_eq!(
            refused(CALENDAR_REPLY, r#"{"v":1,"source":"dav","seq":1}"#),
            PimdirActionError::MissingField("partstat")
        );
        assert_eq!(
            refused(
                CALENDAR_CANCEL,
                r#"{"v":1,"source":"dav","seq":1,"partstat":"ACCEPTED"}"#
            ),
            PimdirActionError::UnknownField("partstat".into())
        );
        assert_eq!(
            refused(CALENDAR_CANCEL, r#"{"v":1,"seq":1}"#),
            PimdirActionError::MissingField("source")
        );

        let pending = r#"{"v":1,"source":"dav","link_id":"uid:a"}"#;
        assert_eq!(
            PimdirInvitation::from_payload(CALENDAR_CANCEL, pending)
                .unwrap()
                .item,
            PimdirIntentItem::Link(PimdirLinkId::from("uid:a"))
        );
    }
}
