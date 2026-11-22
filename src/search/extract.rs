//! # Extraction
//!
//! A body's text by field (SEARCH §6), the input of the index's
//! `object_text` and `summary_text` rows and of the snippets re-read for
//! the hits shown.
//!
//! Each domain reads its bodies behind its feature; a body of a domain
//! left out is read from its summary, as a file is.

#[cfg(feature = "calendar")]
mod calendar;
#[cfg(feature = "contact")]
mod contact;
#[cfg(feature = "mail")]
pub(crate) mod mail;

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::ops::Range;

use crate::{
    collection,
    search::chunk,
    summary::{PimdirAddress, PimdirAddressRole, PimdirSummary},
};

/// The revision of SEARCH §6 this module extracts under and `occurrence`
/// expands under (SEARCH §2, §6), recorded in `index_meta.extraction`: the
/// standard bumps it when what §6 extracts or §7 expands changes, and
/// this constant follows the text vendored.
pub const EXTRACTION: i64 = 1;

/// This crate's own revision of what SEARCH §6 leaves to an indexer
/// (SEARCH §2), recorded in `index_meta.extractor` and compared byte for
/// byte: an index recording another is extracted again.
///
/// Bump it in the change that alters what [`PimdirExtract::body`] or the
/// expansion yields for any body under one [`EXTRACTION`]: the reduction
/// of `text/html` (`html`), the charsets decoded and how, what a field
/// holds, and a bump of mime-rs, vcard-rs, ical-rs or encoding_rs that
/// reads a body otherwise. Revision 1 is the first recorded: tags
/// separating words, and the hidden elements told by their first word.
/// Revision 2: an HTML comment closed by any run of two dashes or more,
/// and a rule stepping past the last year a date spells not cut.
/// Revision 3: a zone read through ical-rs 0.7.3's bounded walk.
pub const EXTRACTOR: &str = "io-pimdir 3";

/// A body's text by field (SEARCH §6): the seven fields, then one per
/// address role (STORAGE Annex A.6).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirExtract {
    /// What the extraction could read.
    pub status: PimdirExtractStatus,
    /// A subject, a name or a summary.
    pub title: String,
    /// Every name and address the item names.
    pub people: String,
    /// The text parts of a message.
    pub body: String,
    /// The part names and media types of a message's attachments.
    pub attachment: String,
    /// A location.
    pub place: String,
    /// An organisation, a title or a role.
    pub org: String,
    /// Notes and telephone numbers.
    pub note: String,
    /// The `from` role's names and addresses.
    pub from: String,
    /// The `to` role's.
    pub to: String,
    /// The `cc` role's.
    pub cc: String,
    /// The `bcc` role's.
    pub bcc: String,
    /// The `email` role's.
    pub email: String,
    /// The `organizer` role's.
    pub organizer: String,
    /// The `attendee` role's.
    pub attendee: String,
    /// Whether a card holds a `TEL`, an `ADR`, a `BDAY`, a `PHOTO`
    /// (`object.tel`, `adr`, `bday`, `photo`); false for any other kind.
    pub tel: bool,
    /// See [`tel`](Self::tel).
    pub adr: bool,
    /// See [`tel`](Self::tel).
    pub bday: bool,
    /// See [`tel`](Self::tel).
    pub photo: bool,
    /// A card's `CATEGORIES` values, folded as STORAGE Annex A.2 folds
    /// `fn`, each once, in document order (`object_category`); none for
    /// any other kind.
    pub categories: Vec<String>,
}

/// What an extraction could read of a body (SEARCH §6).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PimdirExtractStatus {
    /// Read whole.
    #[default]
    Ok,
    /// An encrypted body: headers read, the body not.
    Encrypted,
    /// No parser accepted it: what could be read, from its summary.
    Unparseable,
}

impl PimdirExtractStatus {
    /// The status as `object.status` spells it.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Encrypted => "encrypted",
            Self::Unparseable => "unparseable",
        }
    }
}

/// The columns free text matches: every column of SEARCH §6.
#[cfg(any(feature = "mail", feature = "contact"))]
pub(crate) const COLUMNS: [&str; 14] = [
    "title",
    "people",
    "body",
    "attachment",
    "place",
    "org",
    "note",
    "from",
    "to",
    "cc",
    "bcc",
    "email",
    "organizer",
    "attendee",
];

/// One chunk of an extracted field (SEARCH §6).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirChunk {
    /// The field chunked: `body` or `note`.
    pub field: &'static str,
    /// The UTF-8 byte range into the field's text.
    pub range: Range<usize>,
}

impl PimdirExtract {
    /// A body's extraction by kind, `None` for a file or a domain left out.
    #[cfg_attr(
        not(any(feature = "mail", feature = "contact", feature = "calendar")),
        allow(unused_variables)
    )]
    pub fn body(kind: &str, body: &[u8]) -> Option<Self> {
        match collection::media_type(kind) {
            #[cfg(feature = "mail")]
            "message/rfc822" => Some(Self::mail(body)),
            #[cfg(feature = "contact")]
            "text/vcard" => Some(Self::contact(body)),
            #[cfg(feature = "calendar")]
            "text/calendar" => Some(Self::calendar(body)),
            _ => None,
        }
    }

    /// The `body` and `note` fields of a body's extraction by kind, as
    /// [`body`](Self::body) extracts them, the others left empty where
    /// reading them costs a pass of their own (a message's addresses and
    /// attachments): what a snippet cuts.
    #[cfg_attr(
        not(any(feature = "mail", feature = "contact", feature = "calendar")),
        allow(unused_variables)
    )]
    pub fn text(kind: &str, body: &[u8]) -> Option<Self> {
        match collection::media_type(kind) {
            #[cfg(feature = "mail")]
            "message/rfc822" => Some(Self::mail_text(body)),
            _ => Self::body(kind, body),
        }
    }

    /// Extracts an item from its summary and address rows: a placement
    /// holding no body (SEARCH §6), a file whatever it holds, and a
    /// contact or a calendar item no parser accepts.
    pub fn summary(
        summary: Option<&PimdirSummary>,
        addresses: &[(PimdirAddressRole, PimdirAddress)],
    ) -> Self {
        let mut extract = Self::default();

        if let Some(summary) = summary {
            extract.title = summary.title().to_string();
            match summary {
                PimdirSummary::Contact(contact) => {
                    extract.org = contact.org.clone().unwrap_or_default();
                }
                PimdirSummary::Event(event) => {
                    extract.place = event.location.clone().unwrap_or_default();
                }
                PimdirSummary::File(file) => {
                    extract.attachment = file.media_type.clone().unwrap_or_default();
                }
                _ => (),
            }
        }

        for (role, address) in addresses {
            let text = person(address.name.as_deref(), &address.address);
            let field = match role {
                PimdirAddressRole::From => &mut extract.from,
                PimdirAddressRole::To => &mut extract.to,
                PimdirAddressRole::Cc => &mut extract.cc,
                PimdirAddressRole::Bcc => &mut extract.bcc,
                PimdirAddressRole::Email => &mut extract.email,
                PimdirAddressRole::Organizer => &mut extract.organizer,
                PimdirAddressRole::Attendee => &mut extract.attendee,
                // NOTE: the index keeps no contact field and SEARCH §6 reads
                // a task's people from ORGANIZER and ATTENDEE alone, so a
                // waited-on person is the store's to seek (summary_value).
                PimdirAddressRole::Contact => continue,
            };
            push(field, &text);
            push(&mut extract.people, &text);
        }

        extract
    }

    /// The chunks of the `body` and `note` fields, in that order.
    pub fn chunks(&self) -> Vec<PimdirChunk> {
        let body = chunk::chunks(&self.body)
            .into_iter()
            .map(|range| PimdirChunk {
                field: "body",
                range,
            });
        let note = chunk::chunks(&self.note)
            .into_iter()
            .map(|range| PimdirChunk {
                field: "note",
                range,
            });
        body.chain(note).collect()
    }

    /// The text of the index column `name` (SEARCH §6), empty for one
    /// never filled.
    #[cfg(any(feature = "mail", feature = "contact"))]
    pub(crate) fn column(&self, name: &str) -> &str {
        match name {
            "title" => &self.title,
            "people" => &self.people,
            "body" => &self.body,
            "attachment" => &self.attachment,
            "place" => &self.place,
            "org" => &self.org,
            "note" => &self.note,
            "from" => &self.from,
            "to" => &self.to,
            "cc" => &self.cc,
            "bcc" => &self.bcc,
            "email" => &self.email,
            "organizer" => &self.organizer,
            "attendee" => &self.attendee,
            _ => "",
        }
    }

    /// The text of a chunked field, `None` for a field never chunked.
    pub fn field(&self, field: &str) -> Option<&str> {
        match field {
            "body" => Some(&self.body),
            "note" => Some(&self.note),
            _ => None,
        }
    }
}

/// One person as indexed: the name, then the address.
fn person(name: Option<&str>, address: &str) -> String {
    match name {
        Some(name) if !name.is_empty() => format!("{name} {address}"),
        _ => address.to_string(),
    }
}

/// Appends a line to a field, skipping an empty one.
fn push(field: &mut String, line: &str) {
    if line.is_empty() {
        return;
    }
    if !field.is_empty() {
        field.push('\n');
    }
    field.push_str(line);
}
