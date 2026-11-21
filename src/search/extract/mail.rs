//! # Mail extraction
//!
//! A `message/rfc822` body read through mime-rs (SEARCH §6).

use alloc::{string::String, vec, vec::Vec};

use mime::{
    codec::charset,
    header::marker::{BCC, CC, FROM, MimeHeaderMarker, SUBJECT, TO},
    tree::{body::MimeBodyNode, cst::MimeCst, entity::MimeEntity, parse::MimeParseOptions},
    value::address::MimeAddressList,
};

use crate::{
    search::{
        extract::{PimdirExtract, PimdirExtractStatus, person, push},
        html,
    },
    summary::mail::{self, MIME_DEPTH_MAX, MIME_ENTITIES_MAX, attachments},
};

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
