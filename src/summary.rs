//! # Summaries
//!
//! What a writer derives from an item before its row reaches the store
//! (pimdir STORAGE Annex A): the key it is filed under, the row of its
//! kind's summary table with the people it names, and its sort key.
//!
//! The store parses no body. A connector derives from the bytes at the
//! `Full` tier through [`derive()`], and at the `Meta` tier builds the
//! kind's summary from what the protocol hands it (an IMAP ENVELOPE),
//! through the same decoding the kind modules expose, so the two tiers
//! agree byte for byte as Annex A requires.

pub mod calendar;
pub mod contact;
pub mod mail;
mod time;

use alloc::{format, string::String, vec::Vec};

use crate::{
    placement::{PimdirLinkId, PimdirSortKey},
    summary::{
        calendar::{PimdirEventSummary, PimdirJournalSummary, PimdirTaskSummary},
        contact::PimdirContactSummary,
        mail::PimdirMailSummary,
    },
};

/// One kind's summary row with the addresses it names (Annex A).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirSummary {
    /// A `message/rfc822` item (Annex A.1).
    Mail(PimdirMailSummary),
    /// A `text/vcard` item (Annex A.2).
    Contact(PimdirContactSummary),
    /// A `text/calendar` `VEVENT` resource (Annex A.3).
    Event(PimdirEventSummary),
    /// A `text/calendar` `VTODO` resource (Annex A.4).
    Task(PimdirTaskSummary),
    /// A `text/calendar` `VJOURNAL` resource (Annex A.5).
    Journal(PimdirJournalSummary),
}

impl PimdirSummary {
    /// The identity hint the content states, or `None` for a derived key.
    pub fn hint(&self) -> Option<&str> {
        match self {
            Self::Mail(mail) => mail.message_id.as_deref(),
            Self::Contact(contact) => contact.uid.as_deref(),
            Self::Event(event) => event.uid.as_deref(),
            Self::Task(task) => task.uid.as_deref(),
            Self::Journal(journal) => journal.uid.as_deref(),
        }
    }

    /// What a listing shows the item as: its subject, name or summary.
    pub fn title(&self) -> &str {
        match self {
            Self::Mail(mail) => &mail.subject,
            Self::Contact(contact) => &contact.full_name,
            Self::Event(event) => &event.summary,
            Self::Task(task) => &task.summary,
            Self::Journal(journal) => &journal.summary,
        }
    }

    /// Every person the item names, by role, in document order (A.6).
    pub fn addresses(&self) -> Vec<(PimdirAddressRole, &PimdirAddress)> {
        let mut out = Vec::new();

        match self {
            Self::Mail(mail) => {
                push(&mut out, PimdirAddressRole::From, &mail.from);
                push(&mut out, PimdirAddressRole::To, &mail.to);
                push(&mut out, PimdirAddressRole::Cc, &mail.cc);
                push(&mut out, PimdirAddressRole::Bcc, &mail.bcc);
            }
            Self::Contact(contact) => push(&mut out, PimdirAddressRole::Email, &contact.emails),
            Self::Event(event) => {
                push(
                    &mut out,
                    PimdirAddressRole::Organizer,
                    event.organizer.as_slice(),
                );
                push(&mut out, PimdirAddressRole::Attendee, &event.attendees);
            }
            Self::Task(task) => {
                push(
                    &mut out,
                    PimdirAddressRole::Organizer,
                    task.organizer.as_slice(),
                );
                push(&mut out, PimdirAddressRole::Attendee, &task.attendees);
            }
            Self::Journal(journal) => {
                push(
                    &mut out,
                    PimdirAddressRole::Organizer,
                    journal.organizer.as_slice(),
                );
                push(&mut out, PimdirAddressRole::Attendee, &journal.attendees);
            }
        }

        out
    }
}

/// Appends every address under one role.
fn push<'a>(
    out: &mut Vec<(PimdirAddressRole, &'a PimdirAddress)>,
    role: PimdirAddressRole,
    addresses: &'a [PimdirAddress],
) {
    for address in addresses {
        out.push((role, address));
    }
}

/// One person an item names: the canonical addr-spec and a display name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirAddress {
    /// The addr-spec alone, lowercased whole (Annex A.6).
    pub address: String,
    /// The display name, decoded, when the item carries one.
    pub name: Option<String>,
}

impl PimdirAddress {
    /// The canonical form of a raw address: the addr-spec alone, the
    /// display name, comments, angle brackets and `mailto:` removed,
    /// lowercased whole (Annex A.6).
    pub fn canonical(raw: &str) -> String {
        let raw = raw.trim();
        let inner = match (raw.find('<'), raw.rfind('>')) {
            (Some(open), Some(close)) if open < close => &raw[open + 1..close],
            _ => raw,
        };
        let inner = inner.trim();
        let inner = inner
            .get(..7)
            .filter(|scheme| scheme.eq_ignore_ascii_case("mailto:"))
            .map_or(inner, |_| &inner[7..]);

        inner.trim().to_lowercase()
    }
}

/// The role an address plays for an item (STORAGE §13, Annex A.6).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Ord, PartialOrd)]
pub enum PimdirAddressRole {
    /// A message's `From`.
    From,
    /// A message's `To`.
    To,
    /// A message's `Cc`.
    Cc,
    /// A message's `Bcc`.
    Bcc,
    /// A card's `EMAIL`.
    Email,
    /// A calendar object's `ORGANIZER`.
    Organizer,
    /// A calendar object's `ATTENDEE`.
    Attendee,
}

impl PimdirAddressRole {
    /// The spelling the `role` column carries.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::From => "from",
            Self::To => "to",
            Self::Cc => "cc",
            Self::Bcc => "bcc",
            Self::Email => "email",
            Self::Organizer => "organizer",
            Self::Attendee => "attendee",
        }
    }

    /// The role a column value names, or `None` for one the format lacks.
    pub fn parse(role: &str) -> Option<Self> {
        match role {
            "from" => Some(Self::From),
            "to" => Some(Self::To),
            "cc" => Some(Self::Cc),
            "bcc" => Some(Self::Bcc),
            "email" => Some(Self::Email),
            "organizer" => Some(Self::Organizer),
            "attendee" => Some(Self::Attendee),
            _ => None,
        }
    }
}

/// Everything a writer derives from one item's bytes.
///
/// Written together: an edit refreshing the body without the key would
/// leave the item sorted where its old start put it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirDerivation {
    /// The key the item is filed under (STORAGE §9): the hint when the
    /// content states one, the kind's fallback otherwise.
    pub link_id: PimdirLinkId,
    /// The summary row, `None` for a component the format has no table for.
    pub summary: Option<PimdirSummary>,
    /// The kind's ordering key, empty when nothing orderable was found.
    pub sort_key: PimdirSortKey,
}

/// Derives from a body of the given kind, or `None` for a media type the
/// format has no conventions for.
///
/// The kind is the collection's declared one (STORAGE §14), matched on
/// the bare media type: `text/vcard; charset=utf-8` reads as `text/vcard`.
pub fn derive(kind: &str, body: &[u8]) -> Option<PimdirDerivation> {
    let kind = kind.split(';').next().unwrap_or_default().trim();

    match kind {
        "message/rfc822" => Some(mail::derive(body)),
        "text/vcard" => Some(contact::derive(body)),
        "text/calendar" => Some(calendar::derive(body)),
        _ => None,
    }
}

/// The `hash:` fallback key of a body stating no identity (Annex A.2):
/// the sixteen lowercase hexadecimal digits of its FNV-1a 64 digest.
pub fn hash_key(body: &[u8]) -> PimdirLinkId {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in body {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }

    PimdirLinkId(format!("hash:{hash:016x}"))
}

/// Unfolds a body into its logical lines (RFC 5322 §2.2.3, RFC 5545 §3.1,
/// RFC 6350 §3.2): a line beginning with a space or a tab continues the
/// one before it. Invalid UTF-8 is replaced, never refused (Annex A.0).
fn unfold(body: &[u8], keep_leading_space: bool) -> Vec<String> {
    let text = String::from_utf8_lossy(body);
    let mut lines: Vec<String> = Vec::new();

    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        match line.strip_prefix([' ', '\t']) {
            Some(rest) if !lines.is_empty() => {
                let last = lines.last_mut().expect("a line to continue");
                if keep_leading_space {
                    last.push(' ');
                }
                last.push_str(rest);
            }
            _ => lines.push(String::from(line)),
        }
    }

    lines
}

/// A mail summary carrying only a subject, for the engine's own tests.
#[cfg(test)]
pub(crate) fn stub(subject: &str) -> PimdirSummary {
    PimdirSummary::Mail(PimdirMailSummary {
        subject: subject.into(),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_canonical_address_is_the_lowercased_addr_spec_alone() {
        assert_eq!(
            PimdirAddress::canonical("Alice Example <Alice@Example.ORG>"),
            "alice@example.org"
        );
        assert_eq!(PimdirAddress::canonical("mailto:Bob@x.y"), "bob@x.y");
        assert_eq!(PimdirAddress::canonical(" carol@x.y "), "carol@x.y");
    }

    #[test]
    fn a_role_round_trips_through_its_column_spelling() {
        for role in [
            PimdirAddressRole::From,
            PimdirAddressRole::To,
            PimdirAddressRole::Cc,
            PimdirAddressRole::Bcc,
            PimdirAddressRole::Email,
            PimdirAddressRole::Organizer,
            PimdirAddressRole::Attendee,
        ] {
            assert_eq!(PimdirAddressRole::parse(role.as_str()), Some(role));
        }
        assert_eq!(PimdirAddressRole::parse("sender"), None);
    }

    #[test]
    fn an_unknown_kind_derives_nothing() {
        assert_eq!(derive("application/json", b"{}"), None);
        assert!(derive("text/vcard; charset=utf-8", b"BEGIN:VCARD\r\nEND:VCARD\r\n").is_some());
    }
}
