//! # Extraction
//!
//! A body's text by field (SEARCH §6), the input of the index's
//! `object_text` and `summary_text` rows and of the snippets re-read for
//! the hits shown.
//!
//! Mail goes through mime-rs: the MIME tree, transfer encodings, charsets,
//! RFC 2047 and RFC 2231, `text/plain` preferred within an alternative and
//! `text/html` reduced to text (`html`), under the bounds the summaries
//! keep ([`MIME_DEPTH_MAX`], [`MIME_ENTITIES_MAX`]). It walks the parts
//! the summaries walk and reads an entity's fields from the occurrence
//! they read, the first (STORAGE Annex A.1): a message stating one twice
//! is parsed from [`mail::first_occurrences`]. A vCard goes through
//! vcard-rs and an iCalendar resource through ical-rs. A file, a placement
//! holding no body and a body no parser accepts are read from their
//! summary and address rows.

use alloc::{
    borrow::Cow,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use core::ops::Range;

use ical::{param::IcalParam, tree::cst::IcalCst, value::IcalValue};
use mime::{
    codec::charset,
    header::marker::{BCC, CC, FROM, MimeHeaderMarker, SUBJECT, TO},
    tree::{body::MimeBodyNode, cst::MimeCst, entity::MimeEntity, parse::MimeParseOptions},
    value::address::MimeAddressList,
};
use vcard::{tree::cst::VcardCst, value::VcardValue};

use crate::{
    search::{chunk, fold, html},
    summary::{
        PimdirAddress, PimdirAddressRole, PimdirSummary,
        mail::{self, MIME_DEPTH_MAX, MIME_ENTITIES_MAX, attachments},
    },
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
pub const EXTRACTOR: &str = "io-pimdir 1";

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
        let cst = parse(body);
        if !restates(&cst.root) {
            return Self::parsed(body, &cst.root);
        }

        // NOTE: mime-rs reads the fields an entity is split and decoded by
        // from their last occurrence, Annex A.1 from their first, so a
        // message stating one twice is parsed again with the later ones
        // cut out: the same parts, read as the summary reads them.
        let once = mail::first_occurrences(body);
        Self::parsed(body, &parse(&once).root)
    }

    /// The extraction of the message `body` parsed to `root`.
    fn parsed(body: &[u8], root: &MimeEntity) -> Self {
        let mut extract = if root.header.fields().next().is_none() {
            let Some(text) = headless(body) else {
                return Self {
                    status: PimdirExtractStatus::Unparseable,
                    ..Default::default()
                };
            };

            Self {
                body: text,
                ..Default::default()
            }
        } else {
            let mut extract = Self {
                title: first::<SUBJECT>(root).unwrap_or_default(),
                ..Default::default()
            };

            let roles = [
                (first::<FROM>(root), &mut extract.from),
                (first::<TO>(root), &mut extract.to),
                (first::<CC>(root), &mut extract.cc),
                (first::<BCC>(root), &mut extract.bcc),
            ];
            for (addresses, field) in roles {
                if let Some(addresses) = addresses {
                    push(field, &names(&addresses));
                }
            }
            for field in [&extract.from, &extract.to, &extract.cc, &extract.bcc] {
                push(&mut extract.people, field);
            }

            if is_encrypted(&media_type(root, false)) {
                extract.status = PimdirExtractStatus::Encrypted;
                return extract;
            }

            let walk = Walk::over(root);
            let mut texts = Vec::new();
            walk.part(Some(0), &mut texts);
            extract.body = texts.join("\n\n");
            extract
        };

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
                "ADR" => {
                    extract.adr = true;
                    push(&mut extract.place, &text);
                }
                "BDAY" => extract.bday = true,
                "PHOTO" => extract.photo = true,
                "CATEGORIES" => {
                    for value in vcard_values(&prop.value) {
                        let folded = fold::value(&value);
                        if !folded.is_empty() && !extract.categories.contains(&folded) {
                            extract.categories.push(folded);
                        }
                    }
                }
                "ORG" | "TITLE" | "ROLE" => push(&mut extract.org, &text),
                "NOTE" => push(&mut extract.note, &text),
                "TEL" => {
                    extract.tel = true;
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
                // NOTE: the index keeps no contact field and SEARCH §6 reads
                // a task's people from ORGANIZER and ATTENDEE alone, so a
                // waited-on person is the store's to seek (item_address).
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

    /// The text of a chunked field, `None` for a field never chunked.
    pub fn field(&self, field: &str) -> Option<&str> {
        match field {
            "body" => Some(&self.body),
            "note" => Some(&self.note),
            _ => None,
        }
    }
}

/// The parts the MIME walk reads, whose text it gathers.
struct Walk<'t, 'a> {
    /// The entities the walk of the parts reads (STORAGE Annex A.1), by id
    /// in document order: an unread one and every part under it give no
    /// text.
    parts: Vec<Part<'t, 'a>>,
}

/// One entity the walk reads.
struct Part<'t, 'a> {
    entity: &'t MimeEntity<'a>,
    /// Its media type, lowercased.
    media: (String, String),
    /// The ids of a multipart's children, `None` for one the walk leaves
    /// unread.
    children: Vec<Option<usize>>,
}

impl<'t, 'a> Walk<'t, 'a> {
    /// The walk of a message's parts as Annex A.1 bounds it: in document
    /// order, a multipart before its children, none deeper than
    /// [`MIME_DEPTH_MAX`] and none after the [`MIME_ENTITIES_MAX`]th, the
    /// message's own counted; a `message/rfc822` part is one, not opened.
    fn over(root: &'t MimeEntity<'a>) -> Self {
        let mut parts: Vec<Part<'t, 'a>> = Vec::new();
        let mut pending = vec![(root, 0usize, None::<(usize, usize)>, false)];

        while let Some((entity, depth, slot, digest)) = pending.pop() {
            if depth > MIME_DEPTH_MAX || parts.len() >= MIME_ENTITIES_MAX {
                continue;
            }

            let id = parts.len();
            if let Some((parent, index)) = slot {
                parts[parent].children[index] = Some(id);
            }

            let children = match &entity.body {
                MimeBodyNode::Multipart(multipart) => &multipart.parts[..],
                _ => &[],
            };
            let media = media_type(entity, digest);
            let digest = media.0 == "multipart" && media.1 == "digest";
            parts.push(Part {
                entity,
                media,
                children: vec![None; children.len()],
            });

            for (index, child) in children.iter().enumerate().rev() {
                pending.push((&child.entity, depth + 1, Some((id, index)), digest));
            }
        }

        Self { parts }
    }

    /// Gathers into `texts` the text of the part `id` when the walk reads
    /// it.
    fn part(&self, id: Option<usize>, texts: &mut Vec<String>) {
        let Some(part) = id.and_then(|id| self.parts.get(id)) else {
            return;
        };

        let entity = part.entity;
        let (ctype, subtype) = &part.media;
        // NOTE: disposed as an attachment as Annex A.1's mark reads it:
        // any disposition type but `inline`, a field holding none stating
        // no disposition.
        let attached = entity.disposition().is_some_and(|disposition| {
            let kind = disposition.kind.trim();
            !kind.is_empty() && !kind.eq_ignore_ascii_case("inline")
        });

        match &entity.body {
            MimeBodyNode::Multipart(_) if subtype == "alternative" => {
                let chosen = part
                    .children
                    .iter()
                    .find(|child| self.has_plain(**child))
                    .or(part.children.last());
                if let Some(child) = chosen {
                    self.part(*child, texts);
                }
            }
            MimeBodyNode::Multipart(_) if is_encrypted(&part.media) => (),
            MimeBodyNode::Multipart(_) => {
                for child in &part.children {
                    self.part(*child, texts);
                }
            }
            MimeBodyNode::Leaf(_) if !attached && ctype == "text" && subtype == "plain" => {
                texts.push(text(entity));
            }
            MimeBodyNode::Leaf(_) if !attached && ctype == "text" && subtype == "html" => {
                texts.push(html::to_text(&text(entity)));
            }
            _ => (),
        }
    }

    /// Whether the part `id` is, or holds among the parts the walk reads,
    /// a `text/plain` part an alternative prefers.
    fn has_plain(&self, id: Option<usize>) -> bool {
        let Some(part) = id.and_then(|id| self.parts.get(id)) else {
            return false;
        };

        match &part.entity.body {
            MimeBodyNode::Leaf(_) => part.media == ("text".into(), "plain".into()),
            MimeBodyNode::Multipart(_) => part.children.iter().any(|child| self.has_plain(*child)),
            _ => false,
        }
    }
}

/// The tree of a message.
fn parse(body: &[u8]) -> MimeCst<'_> {
    // NOTE: the walk bounds the entities it reads, the parse none, so a
    // multipart holding the 10,000th entity is still split.
    let opts = MimeParseOptions {
        max_entities: Some(usize::MAX),
        ..Default::default()
    };
    MimeCst::parse_with(body, opts)
}

/// Whether an entity of the tree states twice a field the tree is split
/// or decoded by: `Content-Type`, `Content-Transfer-Encoding` or
/// `Content-Disposition`, named as the summaries name a field, its name
/// trimmed and lowercased. The parts of a `message/rfc822` part, which
/// the walk does not open, are not looked at.
fn restates(root: &MimeEntity) -> bool {
    let mut pending = vec![root];
    while let Some(entity) = pending.pop() {
        let mut stated = [0usize; 3];
        for field in entity.header.fields() {
            let name = field.name_str().trim().to_lowercase();
            let fields = [
                "content-type",
                "content-transfer-encoding",
                "content-disposition",
            ];
            if let Some(at) = fields.iter().position(|field| *field == name) {
                stated[at] += 1;
            }
        }
        if stated.iter().any(|count| *count > 1) {
            return true;
        }

        if let MimeBodyNode::Multipart(multipart) = &entity.body {
            pending.extend(multipart.parts.iter().map(|part| &part.entity));
        }
    }
    false
}

/// The first field the marker names, in document order, read in its
/// form: a field present twice is read from its first occurrence, as
/// STORAGE Annex A.1 reads one (SEARCH §6), so the index holds the
/// subject and the people the summary row states.
pub(crate) fn first<M: MimeHeaderMarker>(entity: &MimeEntity) -> Option<M::Value> {
    let field = entity.header.get_all(&M::KIND).next()?;
    M::from_value(field.read_as(M::KIND.spec().form))
}

/// The text of a message holding no header field: what follows its first
/// line of blanks alone, as UTF-8; `None` when no such line ends its first
/// block.
fn headless(body: &[u8]) -> Option<String> {
    let mut rest = body;

    loop {
        let end = rest.iter().position(|byte| *byte == b'\n')?;
        let (line, next) = rest.split_at(end + 1);
        rest = next;

        if line[..end].iter().all(u8::is_ascii_whitespace) {
            return Some(String::from_utf8_lossy(rest).into_owned());
        }
    }
}

/// A leaf's text: transfer-decoded and transcoded from its `charset`, UTF-8
/// without one; no byte order mark is sniffed and flowed text is kept as
/// written.
fn text(entity: &MimeEntity) -> String {
    let Some(bytes) = entity.decoded() else {
        return String::new();
    };
    let label = entity.content_type().param("charset");

    // NOTE: a US-ASCII label reads as windows-1252, as the WHATWG Encoding
    // Standard reads it, so the 8-bit bytes of a mislabelled part keep their
    // letters.
    let label = match label.as_deref() {
        Some(label)
            if charset::is_utf8_label(label)
                && !charset::clean_label(label)
                    .to_ascii_lowercase()
                    .contains("utf") =>
        {
            "windows-1252"
        }
        Some(label) => label,
        None => "utf-8",
    };

    match charset::transcode(label, &bytes) {
        Some(decoded) => decoded.text.into_owned(),
        None => String::from_utf8_lossy(&bytes).into_owned(),
    }
}

/// An entity's media type, lowercased: when it states none, `text/plain`,
/// or `message/rfc822` for a part of a `multipart/digest` (`digest`).
///
/// Refs: <https://datatracker.ietf.org/doc/html/rfc2046#section-5.1.5>
fn media_type(entity: &MimeEntity, digest: bool) -> (String, String) {
    let media = entity.content_type();

    // NOTE: the `text/plain` an entity stating no type reads as is the only
    // one with no raw value.
    if digest && media.raw.is_empty() {
        return ("message".into(), "rfc822".into());
    }

    (
        String::from_utf8_lossy(media.top).to_ascii_lowercase(),
        String::from_utf8_lossy(media.sub).to_ascii_lowercase(),
    )
}

/// Whether a media type is an encrypted body's (SEARCH §6):
/// `multipart/encrypted` or `application/pkcs7-mime`.
fn is_encrypted(media: &(String, String)) -> bool {
    matches!(
        media,
        (ctype, subtype) if (ctype == "multipart" && subtype == "encrypted")
            || (ctype == "application" && matches!(subtype.as_str(), "pkcs7-mime" | "x-pkcs7-mime"))
    )
}

/// The names and addresses of an address field, one per line.
fn names(addresses: &MimeAddressList) -> String {
    let mut text = String::new();
    for mailbox in addresses.mailboxes() {
        push(
            &mut text,
            &person(
                mailbox.name.as_deref(),
                mailbox.address.as_deref().unwrap_or_default(),
            ),
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

/// A vCard value's items: a list's values, a text as one, anything else
/// none (the values of a `CATEGORIES`, RFC 6350 §6.7.1).
fn vcard_values(value: &VcardValue) -> Vec<String> {
    match value {
        VcardValue::TextList(list) => list.0.iter().map(|value| value.to_string()).collect(),
        VcardValue::Text(value) => vec![value.0.to_string()],
        _ => Vec::new(),
    }
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
