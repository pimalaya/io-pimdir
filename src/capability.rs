//! # Capabilities
//!
//! What a source can push or perform (STORAGE §15.6, Annex B): the
//! vocabulary, one source's declaration, the capabilities an action
//! needs and the check a producer and the owner run it through. I/O-free.

use core::fmt;

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::{String, ToString},
    vec::Vec,
};

use crate::{
    codec::PimdirAction,
    placement::PimdirFlags,
    summary::{PimdirSummary, calendar},
};

/// Adding a message to a collection (Annex B.1).
pub const MAIL_MESSAGE_ADD: &str = "mail.message.add";
/// Copying a message into another collection.
pub const MAIL_MESSAGE_COPY: &str = "mail.message.copy";
/// Moving a message into another collection.
pub const MAIL_MESSAGE_MOVE: &str = "mail.message.move";
/// Removing a message from a collection.
pub const MAIL_MESSAGE_REMOVE: &str = "mail.message.remove";
/// A change of `\Seen`.
pub const MAIL_FLAGS_SEEN: &str = "mail.flags.seen";
/// A change of `\Flagged`.
pub const MAIL_FLAGS_FLAGGED: &str = "mail.flags.flagged";
/// A change of `\Answered`.
pub const MAIL_FLAGS_ANSWERED: &str = "mail.flags.answered";
/// A change of `\Draft`.
pub const MAIL_FLAGS_DRAFT: &str = "mail.flags.draft";
/// A change of `$Important` (RFC 8457), outside the base set: read only
/// where a row states it, [`MAIL_FLAGS_KEYWORDS`] gating it for a source
/// stating none (Annex B.1).
pub const MAIL_FLAGS_IMPORTANT: &str = "mail.flags.important";
/// A change of `$Junk` (RFC 9051 §2.3.2), read as
/// [`MAIL_FLAGS_IMPORTANT`] is.
pub const MAIL_FLAGS_JUNK: &str = "mail.flags.junk";
/// A change of any other flag, and of `$Important` or `$Junk` for a
/// source stating no row for theirs.
pub const MAIL_FLAGS_KEYWORDS: &str = "mail.flags.keywords";
/// Sending a message, the `submit` intent (Annex B.2).
pub const MAIL_SUBMIT: &str = "mail.submit";
/// Filing the copy a `submit` asks for once the message is sent.
pub const MAIL_SUBMIT_COPY: &str = "mail.submit.copy";

/// Creating a collection on the source's server, the `collection-create`
/// intent (Annex B.2): its domain, `collection`, holds for every kind, so
/// it is declared whatever kinds the source syncs.
pub const COLLECTION_CREATE: &str = "collection.create";
/// Renaming the collection the action is queued on, the
/// `collection-rename` intent (Annex B.2), held for every kind as
/// [`COLLECTION_CREATE`] is.
pub const COLLECTION_RENAME: &str = "collection.rename";
/// Deleting the collection the action is queued on with its items, the
/// `collection-delete` intent, held for every kind.
pub const COLLECTION_DELETE: &str = "collection.delete";

/// Every capability of Annex B's base set in the mail domain, what a
/// declaration of a mail source writes whole (§15.6):
/// [`MAIL_FLAGS_IMPORTANT`] and [`MAIL_FLAGS_JUNK`] are written only where
/// the owner knows them.
pub const MAIL: &[&str] = &[
    MAIL_MESSAGE_ADD,
    MAIL_MESSAGE_COPY,
    MAIL_MESSAGE_MOVE,
    MAIL_MESSAGE_REMOVE,
    MAIL_FLAGS_SEEN,
    MAIL_FLAGS_FLAGGED,
    MAIL_FLAGS_ANSWERED,
    MAIL_FLAGS_DRAFT,
    MAIL_FLAGS_KEYWORDS,
    MAIL_SUBMIT,
    MAIL_SUBMIT_COPY,
    COLLECTION_CREATE,
    COLLECTION_RENAME,
    COLLECTION_DELETE,
];

/// Adding a card to an address book.
pub const CONTACTS_CARD_ADD: &str = "contacts.card.add";
/// Replacing a card's body.
pub const CONTACTS_CARD_UPDATE: &str = "contacts.card.update";
/// Removing a card.
pub const CONTACTS_CARD_REMOVE: &str = "contacts.card.remove";
/// Moving a card into another address book.
pub const CONTACTS_CARD_MOVE: &str = "contacts.card.move";
/// Copying a card into another address book.
pub const CONTACTS_CARD_COPY: &str = "contacts.card.copy";

/// Every capability of Annex B's base set in the contacts domain.
pub const CONTACTS: &[&str] = &[
    CONTACTS_CARD_ADD,
    CONTACTS_CARD_UPDATE,
    CONTACTS_CARD_REMOVE,
    CONTACTS_CARD_MOVE,
    CONTACTS_CARD_COPY,
    COLLECTION_CREATE,
    COLLECTION_RENAME,
    COLLECTION_DELETE,
];

/// Adding a calendar resource.
pub const CALENDAR_ITEM_ADD: &str = "calendar.item.add";
/// Replacing a calendar resource's body.
pub const CALENDAR_ITEM_UPDATE: &str = "calendar.item.update";
/// Removing a calendar resource.
pub const CALENDAR_ITEM_REMOVE: &str = "calendar.item.remove";
/// Moving a calendar resource into another calendar.
pub const CALENDAR_ITEM_MOVE: &str = "calendar.item.move";
/// Copying a calendar resource into another calendar.
pub const CALENDAR_ITEM_COPY: &str = "calendar.item.copy";
/// An update changing a component carrying `RECURRENCE-ID`.
pub const CALENDAR_OCCURRENCE_UPDATE: &str = "calendar.occurrence.update";
/// Notifying the attendees of a scheduled resource pushed.
pub const CALENDAR_SCHEDULING: &str = "calendar.scheduling";
/// Creating an online meeting with the resource that asks for one.
pub const CALENDAR_ONLINE_MEETING: &str = "calendar.online-meeting";
/// Whether a calendar collection takes VTODO resources (STORAGE §15.6):
/// outside the base set, read only where a row states it, `none` meaning
/// the collection refuses tasks and no row refusing nothing.
pub const CALENDAR_COMPONENT_VTODO: &str = "calendar.component.vtodo";
/// Replying to an invitation, the `calendar-reply` intent.
pub const CALENDAR_REPLY: &str = "calendar.reply";
/// Cancelling an event the account organises, the `calendar-cancel` intent.
pub const CALENDAR_CANCEL: &str = "calendar.cancel";
/// Replying for one occurrence, a `calendar-reply` naming `recurrence_id`.
pub const CALENDAR_REPLY_OCCURRENCE: &str = "calendar.reply.occurrence";
/// Cancelling one occurrence, a `calendar-cancel` naming `recurrence_id`.
pub const CALENDAR_CANCEL_OCCURRENCE: &str = "calendar.cancel.occurrence";

/// Every capability of Annex B's base set in the calendar domain, what a
/// declaration of a calendar source writes whole (§15.6):
/// [`CALENDAR_COMPONENT_VTODO`] is written only where the owner knows it.
pub const CALENDAR: &[&str] = &[
    CALENDAR_ITEM_ADD,
    CALENDAR_ITEM_UPDATE,
    CALENDAR_ITEM_REMOVE,
    CALENDAR_ITEM_MOVE,
    CALENDAR_ITEM_COPY,
    CALENDAR_OCCURRENCE_UPDATE,
    CALENDAR_SCHEDULING,
    CALENDAR_ONLINE_MEETING,
    CALENDAR_REPLY,
    CALENDAR_CANCEL,
    CALENDAR_REPLY_OCCURRENCE,
    CALENDAR_CANCEL_OCCURRENCE,
    COLLECTION_CREATE,
    COLLECTION_RENAME,
    COLLECTION_DELETE,
];

/// How well a source supports a capability (STORAGE §13).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PimdirSupport {
    /// Supported as the vocabulary defines it.
    Full,
    /// Supported in part, the detail saying what is missing.
    Partial,
    /// Not supported, the detail saying why.
    None,
}

impl PimdirSupport {
    /// The support as its column spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Partial => "partial",
            Self::None => "none",
        }
    }

    /// The inverse of [`as_str`](Self::as_str); an unknown spelling is `None`.
    pub fn parse(value: &str) -> Self {
        match value {
            "full" => Self::Full,
            "partial" => Self::Partial,
            _ => Self::None,
        }
    }
}

/// One row of a source's declaration (§15.6).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirCapability {
    /// The collection the row overrides the source's row in, `None` for
    /// the source-wide row.
    pub collection: Option<String>,
    /// The Annex B name, or an application's own starting with `x-`.
    pub name: String,
    /// How well the source supports it.
    pub support: PimdirSupport,
    /// What a human reads about it.
    pub detail: Option<String>,
}

/// One source's capabilities where an action lands, as a producer reads
/// them: `None` for an undeclared source, which nothing gates.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirSourceCapabilities {
    /// The source.
    pub source: String,
    /// Its rows by name, the collection's override already applied.
    pub declared: Option<BTreeMap<String, (PimdirSupport, Option<String>)>>,
}

/// A capability a source supports in part, which passes and is shown.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirPartial {
    /// The capability.
    pub capability: String,
    /// The source declaring it.
    pub source: String,
    /// What is missing.
    pub detail: Option<String>,
}

impl fmt::Display for PimdirPartial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Source {} supports {} in part",
            self.source, self.capability
        )?;
        if let Some(detail) = &self.detail {
            write!(f, ": {detail}")?;
        }
        Ok(())
    }
}

/// A capability a declared source does not support (§15.6).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirRefusal {
    /// The capability the action needs.
    pub capability: String,
    /// The source lacking it.
    pub source: String,
    /// Why, as the source declared it.
    pub detail: Option<String>,
}

impl fmt::Display for PimdirRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Source {} does not support {}",
            self.source, self.capability
        )?;
        if let Some(detail) = &self.detail {
            write!(f, ": {detail}")?;
        }
        Ok(())
    }
}

impl core::error::Error for PimdirRefusal {}

/// Whether `name` follows Annex B's grammar: lowercase words of letters,
/// digits and hyphens joined by periods, the first beginning with a
/// letter, as `set-performer`'s schema reads a capability (§15.3). An
/// application's own begins with `x-`.
pub fn is_name(name: &str) -> bool {
    let mut words = name.split('.');
    let first = words.next().unwrap_or_default();
    let word = |word: &str| {
        !word.is_empty()
            && word
                .bytes()
                .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'))
    };

    first.starts_with(|char: char| char.is_ascii_lowercase()) && word(first) && words.all(word)
}

/// The capability an intent kind is bound to (Annex B.2), `None` for a
/// kind outside the vocabulary.
pub fn intent_capability(kind: &str) -> Option<&'static str> {
    match kind {
        "submit" => Some(MAIL_SUBMIT),
        "calendar-reply" => Some(CALENDAR_REPLY),
        "calendar-cancel" => Some(CALENDAR_CANCEL),
        "collection-create" => Some(COLLECTION_CREATE),
        "collection-rename" => Some(COLLECTION_RENAME),
        "collection-delete" => Some(COLLECTION_DELETE),
        _ => None,
    }
}

/// The capabilities a calendar write needs from its resources (Annex
/// B.1): `current` is the item's body before the action, `new` the one
/// an `add` or `update` writes. Nothing outside calendars.
pub fn required_by_content(
    kind: &str,
    action: &PimdirAction,
    current: Option<&[u8]>,
    new: Option<&[u8]>,
) -> Vec<String> {
    if kind != "text/calendar" {
        return Vec::new();
    }

    let mut needed = Vec::new();
    let scheduled = match action {
        PimdirAction::Add { .. } | PimdirAction::Update { .. } => new,
        PimdirAction::Remove { .. } => current,
        _ => None,
    };
    if scheduled.is_some_and(calendar::scheduled) {
        needed.push(CALENDAR_SCHEDULING.to_string());
    }

    if let PimdirAction::Add { .. } | PimdirAction::Update { .. } = action
        && new.is_some_and(calendar::online_meeting)
        && !current.is_some_and(calendar::online_meeting)
    {
        needed.push(CALENDAR_ONLINE_MEETING.to_string());
    }

    if let (PimdirAction::Update { .. }, Some(current), Some(new)) = (action, current, new)
        && calendar::occurrences(current) != calendar::occurrences(new)
    {
        needed.push(CALENDAR_OCCURRENCE_UPDATE.to_string());
    }

    needed
}

/// The capabilities a state action needs in a collection of `kind`
/// (Annex B.1). `current` is the item's flag set, against which a
/// `set-flags` is a change; an `add` carries its flags into an empty set
/// and a `copy` the item's own. An intent needs its own capability from
/// its performer alone, so it is left to [`intent_capability`].
pub fn required(kind: &str, action: &PimdirAction, current: &PimdirFlags) -> Vec<String> {
    // NOTE: a collection of no declared kind has no vocabulary, and the
    // drain parks what it cannot stage there anyway.
    let Some((domain, object)) = domain(kind) else {
        return Vec::new();
    };
    let verb = |verb: &str| [domain, object, verb].join(".");

    let empty = BTreeSet::new();
    let current = current.known().unwrap_or(&empty);
    let mut needed = Vec::new();

    match action {
        PimdirAction::Add { flags, .. } => {
            needed.push(verb("add"));
            needed.extend(flag_changes(kind, &empty, flags.known().unwrap_or(&empty)));
        }
        PimdirAction::Copy { .. } => {
            needed.push(verb("copy"));
            needed.extend(flag_changes(kind, &empty, current));
        }
        PimdirAction::Move { .. } => needed.push(verb("move")),
        PimdirAction::Remove { .. } => needed.push(verb("remove")),
        PimdirAction::Update { .. } if domain != "mail" => needed.push(verb("update")),
        PimdirAction::SetFlags { flags, .. } => {
            if let Some(flags) = flags.known() {
                needed.extend(flag_changes(kind, current, flags));
            }
        }
        PimdirAction::Update { .. }
        | PimdirAction::SetPerformer { .. }
        | PimdirAction::Unknown { .. } => {}
    }

    needed.sort();
    needed.dedup();
    needed
}

/// Checks the capabilities an action needs against the sources concerned:
/// a declared source lacking one, or declaring it `none`, refuses the
/// action; a `partial` one passes and is returned to be shown.
pub fn check(
    needed: &[String],
    sources: &[PimdirSourceCapabilities],
) -> Result<Vec<PimdirPartial>, PimdirRefusal> {
    let mut partials = Vec::new();
    for source in sources {
        let Some(declared) = &source.declared else {
            continue;
        };
        for capability in needed {
            let (capability, row) = gating(declared, capability);
            match row {
                Some((PimdirSupport::Full, _)) => {}
                Some((PimdirSupport::Partial, detail)) => partials.push(PimdirPartial {
                    capability: capability.to_string(),
                    source: source.source.clone(),
                    detail: detail.clone(),
                }),
                Some((PimdirSupport::None, detail)) => {
                    return Err(PimdirRefusal {
                        capability: capability.to_string(),
                        source: source.source.clone(),
                        detail: detail.clone(),
                    });
                }
                None => {
                    return Err(PimdirRefusal {
                        capability: capability.to_string(),
                        source: source.source.clone(),
                        detail: None,
                    });
                }
            }
        }
    }
    Ok(partials)
}

/// The row of a declaration gating `capability`, with the name it is
/// read under (Annex B.1): a change of `$Important` or `$Junk` is gated
/// by its own row where the source states one, `none` included, and by
/// [`MAIL_FLAGS_KEYWORDS`] where it states none, so a declaration
/// written before the two reads as it did.
pub fn gating<'a>(
    declared: &'a BTreeMap<String, (PimdirSupport, Option<String>)>,
    capability: &'a str,
) -> (&'a str, Option<&'a (PimdirSupport, Option<String>)>) {
    match declared.get(capability) {
        None if capability == MAIL_FLAGS_IMPORTANT || capability == MAIL_FLAGS_JUNK => {
            (MAIL_FLAGS_KEYWORDS, declared.get(MAIL_FLAGS_KEYWORDS))
        }
        row => (capability, row),
    }
}

/// Whether a calendar resource is a VTODO one, the component its master is
/// (Annex A.3), which [`CALENDAR_COMPONENT_VTODO`] gates.
pub fn is_vtodo(body: &[u8]) -> bool {
    matches!(calendar::derive(body).summary, Some(PimdirSummary::Task(_)))
}

/// Checks that every source concerned takes a VTODO resource where it
/// lands (STORAGE §15.6): one stating [`CALENDAR_COMPONENT_VTODO`] `none`
/// there, or source-wide, refuses it; one stating nothing, declared or
/// not, does not.
pub fn check_vtodo(sources: &[PimdirSourceCapabilities]) -> Result<(), PimdirRefusal> {
    for source in sources {
        if let Some((PimdirSupport::None, detail)) = source
            .declared
            .as_ref()
            .and_then(|declared| declared.get(CALENDAR_COMPONENT_VTODO))
        {
            return Err(PimdirRefusal {
                capability: CALENDAR_COMPONENT_VTODO.to_string(),
                source: source.source.clone(),
                detail: detail.clone(),
            });
        }
    }
    Ok(())
}

/// The domain and object of a collection kind (Annex B).
fn domain(kind: &str) -> Option<(&'static str, &'static str)> {
    match kind {
        "message/rfc822" => Some(("mail", "message")),
        "text/vcard" => Some(("contacts", "card")),
        "text/calendar" => Some(("calendar", "item")),
        _ => None,
    }
}

/// The flag capabilities a move from `before` to `after` needs, every
/// flag added or removed (Annex B.1); none outside mail.
fn flag_changes(kind: &str, before: &BTreeSet<String>, after: &BTreeSet<String>) -> Vec<String> {
    if kind != "message/rfc822" {
        return Vec::new();
    }

    before
        .symmetric_difference(after)
        .map(|flag| match flag.to_ascii_lowercase().as_str() {
            "\\seen" => MAIL_FLAGS_SEEN,
            "\\flagged" => MAIL_FLAGS_FLAGGED,
            "\\answered" => MAIL_FLAGS_ANSWERED,
            "\\draft" => MAIL_FLAGS_DRAFT,
            "$important" => MAIL_FLAGS_IMPORTANT,
            "$junk" => MAIL_FLAGS_JUNK,
            _ => MAIL_FLAGS_KEYWORDS,
        })
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use alloc::{collections::BTreeMap, string::ToString, vec, vec::Vec};

    use super::*;
    use crate::collection::PimdirCollectionId;

    const MAIL_KIND: &str = "message/rfc822";

    fn flags(items: &[&str]) -> PimdirFlags {
        items.iter().collect()
    }

    fn declared(source: &str, rows: &[(&str, PimdirSupport)]) -> PimdirSourceCapabilities {
        let rows: BTreeMap<_, _> = rows
            .iter()
            .map(|(name, support)| (name.to_string(), (*support, Some("why".to_string()))))
            .collect();
        PimdirSourceCapabilities {
            source: source.to_string(),
            declared: Some(rows),
        }
    }

    #[test]
    fn a_move_needs_the_move_capability() {
        let action = PimdirAction::Move {
            seq: 1,
            to: PimdirCollectionId::from("Archive"),
        };

        assert_eq!(
            required(MAIL_KIND, &action, &flags(&["\\Seen"])),
            vec![MAIL_MESSAGE_MOVE.to_string()]
        );
    }

    #[test]
    fn a_set_flags_needs_only_what_it_changes() {
        let action = PimdirAction::SetFlags {
            seq: 1,
            flags: flags(&["\\Seen", "\\Flagged"]),
        };

        assert_eq!(
            required(MAIL_KIND, &action, &flags(&["\\Seen", "$Label"])),
            vec![
                MAIL_FLAGS_FLAGGED.to_string(),
                MAIL_FLAGS_KEYWORDS.to_string()
            ]
        );
    }

    /// `$Important` and `$Junk` name their own capability, whatever their
    /// case (Annex B.1).
    #[test]
    fn important_and_junk_name_their_own_capability() {
        let action = PimdirAction::SetFlags {
            seq: 1,
            flags: flags(&["$important", "$Junk"]),
        };

        assert_eq!(
            required(MAIL_KIND, &action, &flags(&[])),
            vec![
                MAIL_FLAGS_IMPORTANT.to_string(),
                MAIL_FLAGS_JUNK.to_string()
            ]
        );
    }

    /// A source with a row for `$Important` is gated by it, `none`
    /// included; one without is gated by `mail.flags.keywords`, so a
    /// declaration written before the row reads as it did.
    #[test]
    fn important_reads_its_own_row_else_the_keywords_one() {
        use PimdirSupport::{Full, None};
        let needed = vec![MAIL_FLAGS_IMPORTANT.to_string()];

        let gmail = declared(
            "gmail",
            &[(MAIL_FLAGS_KEYWORDS, None), (MAIL_FLAGS_IMPORTANT, Full)],
        );
        assert!(check(&needed, &[gmail]).is_ok());

        let imap = declared("imap", &[(MAIL_FLAGS_KEYWORDS, Full)]);
        assert!(check(&needed, &[imap]).is_ok());

        let graph = declared(
            "graph",
            &[(MAIL_FLAGS_KEYWORDS, Full), (MAIL_FLAGS_IMPORTANT, None)],
        );
        let refusal = check(&needed, &[graph]).unwrap_err();
        assert_eq!(refusal.capability, MAIL_FLAGS_IMPORTANT);

        let older = declared("older", &[(MAIL_FLAGS_KEYWORDS, None)]);
        let refusal = check(&needed, &[older]).unwrap_err();
        assert_eq!(refusal.capability, MAIL_FLAGS_KEYWORDS);
        assert_eq!(refusal.source, "older");
    }

    #[test]
    fn an_add_needs_the_flags_it_carries() {
        let action = PimdirAction::Add {
            link_id: None,
            flags: flags(&["\\Draft"]),
            object: None,
            supersedes: None,
        };

        assert_eq!(
            required(MAIL_KIND, &action, &PimdirFlags::Unknown),
            vec![MAIL_FLAGS_DRAFT.to_string(), MAIL_MESSAGE_ADD.to_string()]
        );
    }

    #[test]
    fn a_declared_source_refuses_what_it_lacks_and_an_undeclared_one_nothing() {
        let needed = vec![MAIL_MESSAGE_MOVE.to_string()];
        let graph = declared("graph", &[(MAIL_MESSAGE_MOVE, PimdirSupport::None)]);
        let bare = declared("bare", &[]);
        let legacy = PimdirSourceCapabilities {
            source: "legacy".to_string(),
            declared: None,
        };

        let refusal = check(&needed, &[legacy.clone(), graph]).unwrap_err();
        assert_eq!(refusal.source, "graph");
        assert_eq!(refusal.detail.as_deref(), Some("why"));
        assert_eq!(check(&needed, &[bare]).unwrap_err().detail, None);
        assert_eq!(check(&needed, &[legacy]).unwrap(), Vec::new());
    }

    #[test]
    fn an_update_changing_an_occurrence_needs_it_and_a_new_stamp_does_not() {
        let series = |stamp: &str, summary: &str| {
            alloc::format!(
                "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:s\r\nRRULE:FREQ=DAILY\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:s\r\nRECURRENCE-ID:20261010T090000Z\r\nDTSTAMP:{stamp}\r\nSUMMARY:{summary}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
            )
        };
        let action = PimdirAction::Update {
            seq: 1,
            object: crate::object::PimdirHash::from("h"),
            if_match: None,
            resolves: None,
        };
        let required = |current: &str, new: &str| {
            required_by_content(
                "text/calendar",
                &action,
                Some(current.as_bytes()),
                Some(new.as_bytes()),
            )
        };

        let before = series("20261001T000000Z", "a");
        assert_eq!(
            required(&before, &series("20261002T000000Z", "a")),
            Vec::<String>::new()
        );
        assert_eq!(
            required(&before, &series("20261001T000000Z", "b")),
            vec![CALENDAR_OCCURRENCE_UPDATE.to_string()]
        );
    }

    #[test]
    fn a_partial_support_passes_and_is_reported() {
        let needed = vec![MAIL_MESSAGE_REMOVE.to_string()];
        let gmail = declared("gmail", &[(MAIL_MESSAGE_REMOVE, PimdirSupport::Partial)]);

        let partials = check(&needed, &[gmail]).unwrap();
        assert_eq!(partials.len(), 1);
        assert_eq!(partials[0].capability, MAIL_MESSAGE_REMOVE);
    }

    /// Annex B names 34 capabilities, each lowercase words joined by
    /// periods, the first its domain, none an application's own.
    #[test]
    fn every_capability_follows_annex_bs_grammar_in_its_domain() {
        let lists = [
            ("mail", MAIL),
            ("contacts", CONTACTS),
            ("calendar", CALENDAR),
            ("calendar", &[CALENDAR_COMPONENT_VTODO]),
            ("mail", &[MAIL_FLAGS_IMPORTANT, MAIL_FLAGS_JUNK]),
        ];

        let mut names = BTreeSet::new();
        for (domain, list) in lists {
            for name in list {
                let first = name.split('.').next().unwrap();
                assert!(first == domain || first == "collection", "{name}");
                assert!(is_name(name) && !name.starts_with("x-"), "{name}");
                names.insert(*name);
            }
        }
        assert_eq!(names.len(), 34);

        for name in ["x-fax.send", "mail.submit.copy", "calendar.online-meeting"] {
            assert!(is_name(name), "{name}");
        }
        for name in [
            "Mail.Submit",
            "mail..submit",
            "mail.",
            ".mail",
            "1mail",
            "",
            "mail.sub mit",
        ] {
            assert!(!is_name(name), "{name:?}");
        }
    }
}
