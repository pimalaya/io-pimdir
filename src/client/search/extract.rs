//! # Extraction
//!
//! A body's text by field (SEARCH §6), the input of the index's
//! `object_text` and `summary_text` rows and of the snippets re-read for
//! the hits shown.
//!
//! Mail goes through mail-parser: the MIME walk, transfer encodings,
//! charsets, RFC 2047 and RFC 2231, `text/plain` preferred within an
//! alternative and `text/html` reduced to text, under the depth bound the
//! summaries keep ([`MIME_DEPTH_MAX`]). A vCard goes through vcard-rs and
//! an iCalendar resource through ical-rs. A file, a placement holding no
//! body and a body no parser accepts are read from their summary and
//! address rows.

use alloc::{
    borrow::Cow,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use core::ops::Range;

use ical::{param::IcalParam, tree::cst::IcalCst, value::IcalValue};
use mail_parser::{
    Address, Message, MessageParser, MimeHeaders, PartType, decoders::html::html_to_text,
};
use vcard::{tree::cst::VcardCst, value::VcardValue};

use crate::{
    search::chunk,
    summary::{
        PimdirAddress, PimdirAddressRole, PimdirSummary,
        mail::{MIME_DEPTH_MAX, attachments},
    },
};

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

/// One chunk of an extracted field (SEARCH §6).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirChunk {
    /// The field chunked: `body` or `note`.
    pub field: &'static str,
    /// The UTF-8 byte range into the field's text.
    pub range: Range<usize>,
}

impl PimdirExtract {
    /// Extracts a `message/rfc822` body. One no parser accepts is
    /// [`Unparseable`](PimdirExtractStatus::Unparseable) with every field
    /// empty, which the caller fills from the summary.
    pub fn mail(body: &[u8]) -> Self {
        let Some(message) = MessageParser::default().parse(body) else {
            return Self {
                status: PimdirExtractStatus::Unparseable,
                ..Default::default()
            };
        };

        let mut extract = Self {
            title: message.subject().unwrap_or_default().to_string(),
            ..Default::default()
        };

        let roles = [
            (message.from(), &mut extract.from),
            (message.to(), &mut extract.to),
            (message.cc(), &mut extract.cc),
            (message.bcc(), &mut extract.bcc),
        ];
        for (address, field) in roles {
            if let Some(address) = address {
                push(field, &names(address));
            }
        }
        for field in [&extract.from, &extract.to, &extract.cc, &extract.bcc] {
            push(&mut extract.people, field);
        }

        if is_encrypted(message.root_part()) {
            extract.status = PimdirExtractStatus::Encrypted;
            return extract;
        }

        let mut walk = Walk::default();
        walk.part(&message, 0, 0);
        extract.body = walk.body.join("\n\n");

        // NOTE: the attachments are the stand-ins' (STORAGE §14.3), so an
        // inline part, an invitation's calendar among them, is none.
        for file in attachments::attachments(body) {
            push(&mut extract.attachment, &file.name);
            push(
                &mut extract.attachment,
                file.media_type.as_deref().unwrap_or_default(),
            );
        }
        extract
    }

    /// Extracts a body of a collection kind (media type) whose content
    /// SEARCH §6 reads: mail, a vCard or an iCalendar resource; `None` for
    /// any other kind, a file's included, which is read from its summary.
    pub fn body(kind: &str, body: &[u8]) -> Option<Self> {
        match kind.split(';').next().unwrap_or_default().trim() {
            "message/rfc822" => Some(Self::mail(body)),
            "text/vcard" => Some(Self::contact(body)),
            "text/calendar" => Some(Self::calendar(body)),
            _ => None,
        }
    }

    /// Extracts a `text/vcard` body through vcard-rs: `FN`, `N` and
    /// `NICKNAME` as the title, every `EMAIL` and the `FN` as people (the
    /// addresses also under `email`), `ADR` as the place, `ORG`, `TITLE`
    /// and `ROLE` as the organisation, `NOTE` and every `TEL`, as written
    /// and as its digits alone, as the note. One no parser accepts is
    /// [`Unparseable`](PimdirExtractStatus::Unparseable) with every field
    /// empty, which the caller fills from the summary.
    pub fn contact(body: &[u8]) -> Self {
        let Ok(cst) = VcardCst::parse(body) else {
            return Self {
                status: PimdirExtractStatus::Unparseable,
                ..Default::default()
            };
        };

        let mut extract = Self::default();
        for prop in cst.decode().properties {
            let name = prop.name.to_ascii_uppercase();
            let text = vcard_text(&prop.value);
            match name.as_str() {
                "FN" => {
                    push(&mut extract.title, &text);
                    push(&mut extract.people, &text);
                }
                "N" | "NICKNAME" => push(&mut extract.title, &text),
                "EMAIL" => {
                    push(&mut extract.email, &text);
                    push(&mut extract.people, &text);
                }
                "ADR" => push(&mut extract.place, &text),
                "ORG" | "TITLE" | "ROLE" => push(&mut extract.org, &text),
                "NOTE" => push(&mut extract.note, &text),
                "TEL" => {
                    let number = text.strip_prefix("tel:").unwrap_or(&text);
                    let digits: String = number.chars().filter(char::is_ascii_digit).collect();
                    push(&mut extract.note, number);
                    if digits != number {
                        push(&mut extract.note, &digits);
                    }
                }
                _ => (),
            }
        }

        extract
    }

    /// Extracts a `text/calendar` body through ical-rs, from every event,
    /// task and journal of the resource, overrides included: `SUMMARY` as
    /// the title, `ORGANIZER` and `ATTENDEE` with their `CN` as people and
    /// under their roles, `DESCRIPTION` as the body, the names `ATTACH`
    /// states as attachments, `LOCATION` as the place and `COMMENT` as the
    /// note. One no parser accepts is
    /// [`Unparseable`](PimdirExtractStatus::Unparseable) with every field
    /// empty, which the caller fills from the summary.
    pub fn calendar(body: &[u8]) -> Self {
        let Ok(cst) = IcalCst::parse(body) else {
            return Self {
                status: PimdirExtractStatus::Unparseable,
                ..Default::default()
            };
        };

        let mut extract = Self::default();
        let calendar = cst.decode();
        let items = calendar.components.iter().filter(|component| {
            matches!(
                component.name.to_ascii_uppercase().as_str(),
                "VEVENT" | "VTODO" | "VJOURNAL"
            )
        });
        for component in items {
            for prop in &component.props {
                let name = prop.name.to_ascii_uppercase();
                let text = ical_text(&prop.value);
                match name.as_str() {
                    "SUMMARY" => push(&mut extract.title, &text),
                    "DESCRIPTION" => push(&mut extract.body, &text),
                    "LOCATION" => push(&mut extract.place, &text),
                    "COMMENT" => push(&mut extract.note, &text),
                    "ORGANIZER" | "ATTENDEE" => {
                        let address = text
                            .get(..7)
                            .filter(|scheme| scheme.eq_ignore_ascii_case("mailto:"))
                            .map_or(text.as_str(), |_| &text[7..]);
                        let cn = prop.params.iter().find_map(|param| match param {
                            IcalParam::Cn(cn) => Some(cn.as_ref()),
                            _ => None,
                        });
                        let text = person(cn, address);
                        let field = match name.as_str() {
                            "ORGANIZER" => &mut extract.organizer,
                            _ => &mut extract.attendee,
                        };
                        push(field, &text);
                        push(&mut extract.people, &text);
                    }
                    // NOTE: its `FILENAME` (RFC 8607) and `FMTTYPE` (SEARCH
                    // §6), so `has:attachment` reads a non-empty field.
                    "ATTACH" => {
                        for param in &prop.params {
                            match param {
                                IcalParam::Unknown { name, values }
                                    if matches!(
                                        name.to_ascii_uppercase().as_str(),
                                        "FILENAME" | "X-FILENAME"
                                    ) =>
                                {
                                    for value in values {
                                        push(&mut extract.attachment, value);
                                    }
                                }
                                IcalParam::FmtType(media_type) => {
                                    push(&mut extract.attachment, media_type);
                                }
                                _ => (),
                            }
                        }
                    }
                    _ => (),
                }
            }
        }

        extract
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

    /// The text of a chunked field, `None` for a field never chunked.
    pub fn field(&self, field: &str) -> Option<&str> {
        match field {
            "body" => Some(&self.body),
            "note" => Some(&self.note),
            _ => None,
        }
    }
}

/// The text parts the MIME walk gathers.
#[derive(Default)]
struct Walk {
    body: Vec<String>,
}

impl Walk {
    /// Walks the part `id` at `depth`, no deeper than [`MIME_DEPTH_MAX`].
    fn part(&mut self, message: &Message, id: usize, depth: usize) {
        if depth >= MIME_DEPTH_MAX {
            return;
        }
        let Some(part) = message.parts.get(id) else {
            return;
        };

        let (ctype, subtype) = media_type(part);
        let attached = part
            .content_disposition()
            .is_some_and(|disposition| disposition.ctype().eq_ignore_ascii_case("attachment"));

        match &part.body {
            PartType::Multipart(children) if subtype == "alternative" => {
                let chosen = children
                    .iter()
                    .find(|child| has_plain(message, **child as usize, depth + 1))
                    .or(children.last());
                if let Some(child) = chosen {
                    self.part(message, *child as usize, depth + 1);
                }
            }
            PartType::Multipart(_) if is_encrypted(part) => (),
            PartType::Multipart(children) => {
                for child in children {
                    self.part(message, *child as usize, depth + 1);
                }
            }
            PartType::Text(text) if !attached && ctype == "text" && subtype == "plain" => {
                self.body.push(text.to_string());
            }
            PartType::Html(html) if !attached => self.body.push(html_to_text(html)),
            _ => (),
        }
    }
}

/// Whether the part `id` is, or holds within the depth bound, a
/// `text/plain` part an alternative prefers.
fn has_plain(message: &Message, id: usize, depth: usize) -> bool {
    if depth >= MIME_DEPTH_MAX {
        return false;
    }
    let Some(part) = message.parts.get(id) else {
        return false;
    };

    match &part.body {
        PartType::Text(_) => media_type(part) == ("text".into(), "plain".into()),
        PartType::Multipart(children) => children
            .iter()
            .any(|child| has_plain(message, *child as usize, depth + 1)),
        _ => false,
    }
}

/// A part's media type, lowercased, `text/plain` when it states none.
fn media_type<'a>(part: &impl MimeHeaders<'a>) -> (String, String) {
    match part.content_type() {
        Some(ctype) => (
            ctype.ctype().to_ascii_lowercase(),
            ctype.subtype().unwrap_or_default().to_ascii_lowercase(),
        ),
        None => ("text".into(), "plain".into()),
    }
}

/// Whether a part is an encrypted body (SEARCH §6): `multipart/encrypted`
/// or `application/pkcs7-mime`.
fn is_encrypted<'a>(part: &impl MimeHeaders<'a>) -> bool {
    matches!(
        media_type(part),
        (ctype, subtype) if (ctype == "multipart" && subtype == "encrypted")
            || (ctype == "application" && matches!(subtype.as_str(), "pkcs7-mime" | "x-pkcs7-mime"))
    )
}

/// The names and addresses of an address header, one per line.
fn names(address: &Address) -> String {
    let mut text = String::new();
    for addr in address.iter() {
        push(
            &mut text,
            &person(addr.name(), addr.address().unwrap_or_default()),
        );
    }
    text
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

/// A vCard value as text: a text, each component of a list or a
/// structured value on its own line, a URI as written.
fn vcard_text(value: &VcardValue) -> String {
    let mut text = String::new();
    let lists: Vec<&Vec<Cow<str>>> = match value {
        VcardValue::Text(value) => return value.0.to_string(),
        VcardValue::Uri(value) => return value.0.to_string(),
        VcardValue::TextList(list) => vec![&list.0],
        VcardValue::Org(org) => vec![&org.0],
        VcardValue::N(n) => vec![&n.prefixes, &n.given, &n.additional, &n.family, &n.suffixes],
        VcardValue::Adr(adr) => vec![
            &adr.po_box,
            &adr.extended,
            &adr.street,
            &adr.locality,
            &adr.region,
            &adr.postal_code,
            &adr.country,
        ],
        _ => Vec::new(),
    };
    for value in lists.into_iter().flatten() {
        push(&mut text, value);
    }
    text
}

/// An iCalendar value as text: a text, a list's items on their own lines,
/// an address or a URI as written.
fn ical_text(value: &IcalValue) -> String {
    match value {
        IcalValue::Text(text) => text.0.to_string(),
        IcalValue::TextList(list) => {
            let mut text = String::new();
            for item in &list.0 {
                push(&mut text, item);
            }
            text
        }
        IcalValue::CalAddress(address) => address.0.to_string(),
        IcalValue::Uri(uri) => uri.0.to_string(),
        _ => String::new(),
    }
}
