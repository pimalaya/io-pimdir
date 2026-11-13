//! # Mail summaries
//!
//! The `message/rfc822` derivation (Annex A.1, revision 3): the header
//! block read as the annex reads it (comments, quoted-strings, RFC 2047
//! encoded words under WHATWG labels, the `Message-ID` and the `Date`
//! forms), the addresses canonical (A.6), and the MIME tree walked for an
//! attachment and an invitation within its bounds.
//!
//! The decoders are public so a connector building the summary from an
//! IMAP ENVELOPE at the `Meta` tier lands on the same bytes the `Full`
//! tier derives from the body, and [`derive_meta`] reads a header block
//! the way a listing names a member, the attachment mark read without
//! the body.

pub mod attachments;

use alloc::{
    borrow::Cow,
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::ops::Range;

use crate::{
    placement::{PimdirLinkId, PimdirSortKey},
    summary::{PimdirAddress, PimdirDerivation, PimdirSummary, time, unfold},
};

/// The deepest multipart nesting the walk of the parts opens, the message
/// itself at zero (Annex A.1).
///
/// A part nested deeper is neither examined for the attachment mark nor
/// for the invitation. No mail agent nests near it, while a sender nesting
/// further would otherwise cost the walk a stack frame and a rescan of the
/// body per level.
pub const MIME_DEPTH_MAX: usize = 64;

/// The most entities the walk of the parts reads, in document order, the
/// message's own counted (Annex A.1): an entity after it, with every part
/// under it, makes no mark, no invitation, no stand-in and no extracted
/// text. Dovecot's default `max_total_mime_parts`.
pub const MIME_ENTITIES_MAX: usize = 10_000;

/// The fields an entity is read by, each from its first occurrence when
/// the entity states it twice (Annex A.1), as [`header_fields`] names
/// them.
const ENTITY_FIELDS: [&str; 4] = [
    "content-type",
    "content-disposition",
    "content-transfer-encoding",
    "content-id",
];

/// The `mail_summary` row of a message with the people it names.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirMailSummary {
    /// The bare `Message-ID`, angle brackets stripped.
    pub message_id: Option<String>,
    /// The `In-Reply-To` ids, bare, in document order.
    pub in_reply_to: Vec<String>,
    /// The `Subject`, decoded; empty when absent.
    pub subject: String,
    /// The first `From` addr-spec, canonical.
    pub sender: Option<String>,
    /// Its display name, decoded.
    pub sender_name: Option<String>,
    /// The `Date` as an RFC 3339 instant in UTC; `None` when unparseable.
    pub date: Option<String>,
    /// The raw message octets, or `RFC822.SIZE` at the `Meta` tier.
    pub size: Option<u64>,
    /// The attachment mark (Annex A.1): from the walk of the parts when
    /// the body was read, else read without it ([`derive_meta`]); `None`
    /// only on a row an earlier draft wrote at the `Meta` tier.
    pub attachment: Option<bool>,
    /// The `UID` the first `text/calendar` part carries (Annex A.1), read
    /// with the body; `None` without it or when no part is one.
    pub invitation: Option<String>,
    /// Every `From` address, in document order.
    pub from: Vec<PimdirAddress>,
    /// Every `To` address, in document order.
    pub to: Vec<PimdirAddress>,
    /// Every `Cc` address, in document order.
    pub cc: Vec<PimdirAddress>,
    /// Every `Bcc` address, in document order.
    pub bcc: Vec<PimdirAddress>,
}

impl PimdirMailSummary {
    /// The key the message is filed under: the `Message-ID`, else `alt:`
    /// over the subject, the date and the sender (Annex A.1).
    pub fn link_id(&self) -> PimdirLinkId {
        match &self.message_id {
            Some(id) => PimdirLinkId::from_hint(id.as_str()),
            None => PimdirLinkId(format!(
                "alt:{}|{}|{}",
                self.subject,
                self.date.as_deref().unwrap_or_default(),
                self.sender.as_deref().unwrap_or_default(),
            )),
        }
    }

    /// The date column, read descending for a newest-first listing.
    pub fn sort_key(&self) -> PimdirSortKey {
        PimdirSortKey(self.date.clone().unwrap_or_default())
    }

    /// The whole derivation this summary yields.
    pub fn derivation(self) -> PimdirDerivation {
        PimdirDerivation {
            link_id: self.link_id(),
            sort_key: self.sort_key(),
            summary: Some(PimdirSummary::Mail(self)),
        }
    }
}

/// Derives a message's key, summary and sort key from its bytes.
///
/// The parts are walked for the attachment mark (Annex A.1), which
/// replaces any mark read without the body, and for the invitation.
pub fn derive(body: &[u8]) -> PimdirDerivation {
    let (headers, rest) = split_headers(body);
    let headers = header_fields(headers);
    let attachment = has_attachment(&headers, rest, 0, &mut 0, false);

    let mut summary = from_headers(&headers, Some(body.len() as u64), attachment);
    summary.invitation = invitation(&headers, rest, 0, &mut 0);
    summary.derivation()
}

/// Derives what names a message without its body (Annex A.1): the same
/// key, summary and sort key [`derive()`] gives, read off the header block
/// alone, the way a listing names a member (SYNC §4).
///
/// `header` is the header block (an IMAP `BODY.PEEK[HEADER]` or
/// `HEADER.FIELDS`, Gmail's metadata headers), or a whole message whose
/// body is ignored. `size` is the server's `RFC822.SIZE`. The attachment
/// mark is `attachment`, the source's own flag where it states one
/// (Graph's `hasAttachments`, JMAP's `hasAttachment`), else
/// [`meta_attachment`] over the top-level `Content-Type`. A connector
/// reading the fields off a server-side summary (an `ENVELOPE`, a Graph
/// `$select`) builds [`PimdirMailSummary`] itself through the decoders
/// this module exposes, and sets the mark the same way.
pub fn derive_meta(header: &[u8], size: Option<u64>, attachment: Option<bool>) -> PimdirDerivation {
    let (headers, _) = split_headers(header);
    let headers = header_fields(headers);
    let attachment = attachment.unwrap_or_else(|| meta_attachment(field(&headers, "content-type")));

    from_headers(&headers, size, attachment).derivation()
}

/// The attachment mark read without the body (Annex A.1): `true` when the
/// top-level `Content-Type` is `multipart/mixed`, `false` otherwise, none
/// included. It misses both ways (a list footer as a second part reads as
/// one, an attachment under `multipart/signed` as none), each corrected
/// when the body is read and the parts walked.
pub fn meta_attachment(content_type: Option<&str>) -> bool {
    content_type
        .and_then(|value| value.split(';').next())
        .is_some_and(|media| media.trim().eq_ignore_ascii_case("multipart/mixed"))
}

/// The message with every field an entity is read by stated once (Annex
/// A.1): the later occurrences of `Content-Type`, `Content-Disposition`,
/// `Content-Transfer-Encoding` and `Content-ID` cut out of the header
/// block of each entity the walk of the parts reads, every other octet as
/// it was. Borrowed when no entity states one twice.
///
/// What a reader hands a parser reading such a field from its last
/// occurrence, so that the parts it splits and the fields it reads are
/// those the derivation reads, from the first (SEARCH §6).
pub fn first_occurrences(body: &[u8]) -> Cow<'_, [u8]> {
    let mut cuts = Vec::new();
    restated(body, 0, 0, &mut 0, &mut cuts);
    if cuts.is_empty() {
        return Cow::Borrowed(body);
    }

    let mut once = Vec::with_capacity(body.len());
    let mut at = 0;
    for cut in cuts {
        once.extend_from_slice(&body[at..cut.start]);
        at = cut.end;
    }
    once.extend_from_slice(&body[at..]);
    Cow::Owned(once)
}

/// The summary a header block names, the attachment mark given.
fn from_headers(
    headers: &[(String, String)],
    size: Option<u64>,
    attachment: bool,
) -> PimdirMailSummary {
    let header = |name: &str| field(headers, name);

    let from = header("from").map(addresses).unwrap_or_default();
    let first = from.first();

    PimdirMailSummary {
        message_id: header("message-id").and_then(message_id),
        in_reply_to: header("in-reply-to").map(message_ids).unwrap_or_default(),
        subject: header("subject").map(decode).unwrap_or_default(),
        sender: first.map(|address| address.address.clone()),
        sender_name: first.and_then(|address| address.name.clone()),
        date: header("date").and_then(instant),
        size,
        attachment: Some(attachment),
        invitation: None,
        to: header("to").map(addresses).unwrap_or_default(),
        cc: header("cc").map(addresses).unwrap_or_default(),
        bcc: header("bcc").map(addresses).unwrap_or_default(),
        from,
    }
}

/// Decodes an unstructured value, a `Subject` or a display name (Annex
/// A.1): every RFC 2047 encoded word decoded wherever it stands.
///
/// A malformed word, or one whose charset no WHATWG label names, stays as
/// its literal text. The whitespace standing between two decoded words is
/// removed (RFC 2047 §6.2) and no other, and the ends are trimmed of
/// spaces and tabs, whitespace inside kept as it is.
pub fn decode(raw: &str) -> String {
    let mut out = String::new();
    let mut text = 0;
    let mut at = 0;
    // NOTE: where the literal text since the last decoded word starts in
    // `out`, dropped when the next word decodes and it is whitespace.
    let mut after_word: Option<usize> = None;

    while let Some(offset) = raw[at..].find("=?") {
        let start = at + offset;
        let Some((decoded, len)) = encoded_word(&raw[start..]) else {
            at = start + 1;
            continue;
        };
        at = start + len;

        // NOTE: a malformed word stays in the literal text around it.
        let Some(decoded) = decoded else {
            continue;
        };
        out.push_str(&raw[text..start]);
        if let Some(gap) = after_word
            && out[gap..].chars().all(is_wsp)
        {
            out.truncate(gap);
        }
        out.push_str(&decoded);
        after_word = Some(out.len());
        text = at;
    }
    out.push_str(&raw[text..]);

    trim_wsp(&out).to_string()
}

/// One RFC 2047 `encoded-word` at the head of `text`, its octets decoded
/// (`None` when malformed or in a charset no WHATWG label names), and its
/// length; `None` when `text` opens on none.
///
/// The charset is an RFC 2047 token, the encoding `B` or `Q` in either
/// case and the encoded text one or more printable ASCII characters other
/// than `?`, a space being none.
fn encoded_word(text: &str) -> Option<(Option<String>, usize)> {
    let especial = |char: char| "()<>@,;:\\\"/[]?.=".contains(char);
    let token = |char: char| char > ' ' && char != '\x7f' && !especial(char);
    let printable = |char: char| ('!'..='~').contains(&char) && char != '?';

    let rest = text.strip_prefix("=?")?;
    let charset = &rest[..rest.find(|char| !token(char)).unwrap_or(rest.len())];
    let rest = rest[charset.len()..].strip_prefix('?')?;
    let encoding = rest.chars().next()?;
    let rest = rest.get(1..)?.strip_prefix('?')?;
    let payload = &rest[..rest.find(|char| !printable(char)).unwrap_or(rest.len())];
    if charset.is_empty() || payload.is_empty() || !rest[payload.len()..].starts_with("?=") {
        return None;
    }

    let octets = match encoding {
        'B' | 'b' => b_octets(payload),
        'Q' | 'q' => q_octets(payload),
        _ => return None,
    };
    let len = 2 + charset.len() + 3 + payload.len() + 2;

    Some((
        octets.and_then(|octets| charset_decode(charset, &octets)),
        len,
    ))
}

/// An encoded word's `B` text as octets (RFC 4648 §4), its trailing `=`
/// optional; `None` for a character outside the alphabet, an `=` anywhere
/// but in a trailing padding of at most two, or a length, padding aside,
/// leaving one character beyond whole groups of four.
fn b_octets(text: &str) -> Option<Vec<u8>> {
    let bare = text.trim_end_matches('=');
    if text.len() - bare.len() > 2 || bare.len() % 4 == 1 {
        return None;
    }

    let mut out = Vec::with_capacity(bare.len() * 3 / 4);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for byte in bare.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }

    Some(out)
}

/// An encoded word's `Q` text as octets (RFC 2047 §4.2): `_` a space and
/// `=` with two hexadecimal digits of either case one octet; `None` for an
/// `=` not followed by two.
fn q_octets(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;

    while at < bytes.len() {
        match bytes[at] {
            b'_' => {
                out.push(b' ');
                at += 1;
            }
            b'=' => {
                let pair = bytes.get(at + 1..at + 3)?;
                out.push(hex_pair(pair[0], pair[1])?);
                at += 3;
            }
            byte => {
                out.push(byte);
                at += 1;
            }
        }
    }

    Some(out)
}

/// The labels of the WHATWG Encoding Standard naming UTF-8.
const UTF_8_LABELS: [&str; 6] = [
    "unicode-1-1-utf-8",
    "unicode11utf8",
    "unicode20utf8",
    "utf-8",
    "utf8",
    "x-unicode20utf8",
];

/// The labels of the WHATWG Encoding Standard naming windows-1252, the
/// encoding it reads `iso-8859-1` and `us-ascii` as.
const WINDOWS_1252_LABELS: [&str; 17] = [
    "ansi_x3.4-1968",
    "ascii",
    "cp1252",
    "cp819",
    "csisolatin1",
    "ibm819",
    "iso-8859-1",
    "iso-ir-100",
    "iso8859-1",
    "iso88591",
    "iso_8859-1",
    "iso_8859-1:1987",
    "l1",
    "latin1",
    "us-ascii",
    "windows-1252",
    "x-cp1252",
];

/// Octets under a MIME charset as text (Annex A.1), or `None` for a
/// charset no label of the WHATWG Encoding Standard names, or one naming
/// its replacement encoding: the label, an RFC 2231 language after `*`
/// dropped, matched without regard to ASCII case, and the octets decoded
/// by the encoding it names, no byte order mark sniffed.
///
/// Every build carries the tables, the legacy indexes among them
/// (ISO-8859-2, KOI8-R, ISO-2022-JP, GBK and the rest), which Annex A.1
/// has a writer carry: a build decoding fewer labels would derive other
/// subjects, display names and `alt:` keys, silently. The UTF-8 and
/// windows-1252 labels are read here, ill-formed UTF-8 replaced (Annex
/// A.0), every other one through `encoding_rs`.
pub fn charset_decode(charset: &str, octets: &[u8]) -> Option<String> {
    let label = charset
        .split('*')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();

    match label.as_str() {
        label if UTF_8_LABELS.contains(&label) => {
            Some(String::from_utf8_lossy(octets).into_owned())
        }
        label if WINDOWS_1252_LABELS.contains(&label) => {
            Some(octets.iter().map(|byte| cp1252(*byte)).collect())
        }
        label => encoding_rs::Encoding::for_label_no_replacement(label.as_bytes())
            .map(|encoding| encoding.decode_without_bom_handling(octets).0.into_owned()),
    }
}

/// One `windows-1252` byte as its code point: latin1 except for the
/// 0x80 to 0x9F row, where five undefined bytes map to themselves.
fn cp1252(byte: u8) -> char {
    const ROW: [char; 32] = [
        '\u{20ac}', '\u{81}', '\u{201a}', '\u{192}', '\u{201e}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{2c6}', '\u{2030}', '\u{160}', '\u{2039}', '\u{152}', '\u{8d}', '\u{17d}',
        '\u{8f}', '\u{90}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}', '\u{2013}',
        '\u{2014}', '\u{2dc}', '\u{2122}', '\u{161}', '\u{203a}', '\u{153}', '\u{9d}', '\u{17e}',
        '\u{178}',
    ];

    match byte {
        0x80..=0x9f => ROW[usize::from(byte - 0x80)],
        _ => char::from(byte),
    }
}

/// Decodes RFC 2045 quoted-printable octet by octet: an octet that is no
/// UTF-8 passes through as itself, replaced only once the decoded text is
/// read (Annex A.0).
fn quoted_printable(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;

    while at < bytes.len() {
        match bytes[at] {
            b'=' if at + 2 < bytes.len() => match hex_pair(bytes[at + 1], bytes[at + 2]) {
                Some(byte) => {
                    out.push(byte);
                    at += 3;
                }
                None => {
                    out.push(b'=');
                    at += 1;
                }
            },
            byte => {
                out.push(byte);
                at += 1;
            }
        }
    }

    out
}

fn hex_pair(high: u8, low: u8) -> Option<u8> {
    let digit = |byte: u8| (byte as char).to_digit(16).map(|digit| digit as u8);
    Some(digit(high)? << 4 | digit(low)?)
}

/// Decodes RFC 4648 base64, ignoring whitespace; `None` on a foreign byte.
fn base64(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut buffer: u32 = 0;
    let mut bits = 0;

    for &byte in bytes {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }

    Some(out)
}

/// The addresses of an address-list header, in document order (Annex
/// A.1): split at the commas outside a quoted-string, a comment and angle
/// brackets, a group listing its own mailboxes.
///
/// A mailbox's address is the text between its angle brackets, else its
/// whole text with its comments removed, made canonical (Annex A.6); its
/// name is its [`display_name`].
pub fn addresses(raw: &str) -> Vec<PimdirAddress> {
    mailboxes(raw)
        .iter()
        .filter_map(|mailbox| {
            let opened = lexemes(mailbox)
                .find(|lexeme| lexeme.char == '<' && lexeme.depth == 0 && !lexeme.quoted)
                .map(|lexeme| lexeme.at)
                .and_then(|open| Some((open, open + 1 + mailbox[open + 1..].find('>')?)));
            let (address, name) = match opened {
                Some((open, close)) => (
                    trim_wsp(&mailbox[open + 1..close]).to_string(),
                    display_name(&mailbox[..open]),
                ),
                None => (trim_wsp(&uncommented(mailbox)).to_string(), None),
            };

            let address = PimdirAddress::canonical(&address);
            (!address.is_empty()).then_some(PimdirAddress { address, name })
        })
        .collect()
}

/// Splits an address-list into its mailboxes: at the commas outside a
/// quoted-string, a comment and angle brackets, a group's name and colon
/// dropped and its closing semicolon splitting.
fn mailboxes(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut angled = false;

    for lexeme in lexemes(raw) {
        if lexeme.depth == 0 && !lexeme.quoted && !lexeme.escaped {
            match lexeme.char {
                '<' | '>' => angled = lexeme.char == '<',
                ':' if !angled && !current.contains('@') => {
                    current.clear();
                    continue;
                }
                ',' | ';' if !angled => {
                    out.push(core::mem::take(&mut current));
                    continue;
                }
                _ => {}
            }
        }
        current.push(lexeme.char);
    }
    out.push(current);

    out
}

/// The display name of a mailbox, the text before its `<` (Annex A.1):
/// its comments read as spaces, each quoted-string its content with its
/// quoted pairs undone, then [`decode`]d; `None` when empty.
pub fn display_name(phrase: &str) -> Option<String> {
    let mut out = String::with_capacity(phrase.len());
    for lexeme in lexemes(phrase) {
        if lexeme.depth > 0 {
            if lexeme.char == '(' && lexeme.depth == 1 && !lexeme.escaped {
                out.push(' ');
            }
        } else if lexeme.escaped || !(lexeme.char == '"' || lexeme.char == '\\' && lexeme.quoted) {
            out.push(lexeme.char);
        }
    }

    Some(decode(&out)).filter(|name| !name.is_empty())
}

/// The bare id a `Message-ID` field names (Annex A.1), its first msg-id,
/// or `None` when unparseable: comments skipped, the text from the first
/// `<` outside one to the next `>`, trimmed and not empty; with no such
/// `<`, the field read bracketless, its comments removed and trimmed,
/// when it holds no whitespace, no `<`, `>` or `"` and exactly one `@`.
pub fn message_id(raw: &str) -> Option<String> {
    ids(raw, true).into_iter().next()
}

/// The bare ids of an `In-Reply-To` or `References` field (Annex A.1), in
/// document order: each `<` outside a comment to the next `>`, trimmed,
/// empty ones dropped; with no such `<`, each whitespace-separated word of
/// the field, its comments removed, that [`message_id`] would take.
pub fn message_ids(raw: &str) -> Vec<String> {
    ids(raw, false)
}

/// The msg-ids of a field, the first alone when `first` (Annex A.1).
fn ids(raw: &str, first: bool) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut opened: Option<usize> = None;

    for lexeme in lexemes(raw) {
        match opened {
            Some(start) if lexeme.char == '>' => {
                found.push(trim_wsp(&raw[start..lexeme.at]).to_string());
                opened = None;
                if first {
                    break;
                }
            }
            None if lexeme.char == '<' && lexeme.depth == 0 => opened = Some(lexeme.at + 1),
            _ => {}
        }
    }
    if !found.is_empty() || opened.is_some() {
        found.retain(|id| !id.is_empty());
        return found;
    }

    let bare = uncommented(raw);
    let words: Vec<&str> = bare
        .split([' ', '\t'])
        .filter(|word| !word.is_empty())
        .collect();
    if first && words.len() != 1 {
        return Vec::new();
    }
    words
        .into_iter()
        .filter(|word| word.matches('@').count() == 1 && !word.contains(['<', '>', '"']))
        .map(String::from)
        .collect()
}

/// An RFC 5322 `date-time` as the RFC 3339 instant in UTC the summary
/// stores (Annex A.1), or `None` when unparseable: a date is never
/// invented.
///
/// RFC 5322 §3.3 with the obsolete forms of §4.3: comments removed, the
/// day of the week before the first comma skipped and never checked, then
/// five words, whitespace around a colon not counting: the day, the month,
/// a year of two digits or more (two below 50 in the 2000s, two from 50 or
/// three in the 1900s, then 1900 to 9999), `hh:mm[:ss]` and the zone. The
/// day exists in its month, the hour is 00 to 23, a second of 60 reads as
/// 59, and nothing is carried into the next day.
pub fn instant(raw: &str) -> Option<String> {
    let text = uncommented(raw);
    let text = text.split_once(',').map_or(text.as_str(), |(_, rest)| rest);

    // NOTE: the whitespace around a colon is no word separator.
    let mut squeezed = String::with_capacity(text.len());
    let mut colon = false;
    for char in text.chars() {
        match char {
            ':' => {
                squeezed.truncate(trim_wsp_end(&squeezed).len());
                squeezed.push(char);
                colon = true;
            }
            ' ' | '\t' if colon => {}
            char => {
                squeezed.push(char);
                colon = false;
            }
        }
    }

    let words: Vec<&str> = squeezed
        .split([' ', '\t'])
        .filter(|word| !word.is_empty())
        .collect();
    let [day, month, year, clock, zone] = words.as_slice() else {
        return None;
    };

    let day = (day.len() <= 2).then(|| digits(day)).flatten()?;
    let month = self::month(month)?;
    let year = self::year(year)?;
    let (hour, minute, second) = match clock.as_bytes() {
        [_, _, b':', _, _] => (
            time::digits(clock.as_bytes(), 0, 2)?,
            time::digits(clock.as_bytes(), 3, 2)?,
            0,
        ),
        [_, _, b':', _, _, b':', _, _] => (
            time::digits(clock.as_bytes(), 0, 2)?,
            time::digits(clock.as_bytes(), 3, 2)?,
            time::digits(clock.as_bytes(), 6, 2)?,
        ),
        _ => return None,
    };
    if !(1..=time::days_in_month(year, month)).contains(&(day as u32))
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }

    let offset = self::zone(zone)?;
    let instant = time::unix(year, month, day as u32, hour, minute, second.min(59)) - offset;
    (instant < time::unix(10_000, 1, 1, 0, 0, 0)).then(|| time::rfc3339(instant))
}

/// A message's `date` when it is one the store takes (STORAGE §13, Annex
/// A.0): an instant in UTC at seconds precision under the `Z` designator
/// (`2026-08-01T10:00:00Z`), digit for digit, as [`instant`] writes one.
/// Any other text, an empty one, a fraction of a second and an offset
/// included, is no usable date: `None`, as an unparseable `Date` is.
pub fn usable_date(date: Option<&str>) -> Option<&str> {
    date.filter(|date| time::is_instant(date))
}

/// The month a `date-time`'s three-letter name stands for, in any case.
fn month(name: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];

    MONTHS
        .iter()
        .position(|month| month.eq_ignore_ascii_case(name))
        .map(|index| index as u32 + 1)
}

/// A `date-time`'s year: two digits or more, the two- and three-digit
/// forms of RFC 5322 §4.3 read in the 1900s and 2000s, then 1900 to 9999.
fn year(raw: &str) -> Option<i32> {
    let year = digits(raw)?;
    let year = match raw.len() {
        0 | 1 => return None,
        2 if year < 50 => 2000 + year,
        2 | 3 => 1900 + year,
        _ => year,
    };

    (1900..=9999).contains(&year).then_some(year as i32)
}

/// A `date-time`'s zone as the seconds to subtract to reach UTC: `+hhmm`
/// or `-hhmm` with `mm` at most 59, a named zone of RFC 5322 §4.3 in any
/// case, or one military letter other than `J`, which §4.3 reads as
/// `-0000` and so at `+0000`.
fn zone(raw: &str) -> Option<i64> {
    let bytes = raw.as_bytes();
    if let [sign @ (b'+' | b'-'), ..] = bytes
        && bytes.len() == 5
    {
        let hours = time::digits(bytes, 1, 2)? as i64;
        let minutes = time::digits(bytes, 3, 2).filter(|minutes| *minutes <= 59)? as i64;
        let offset = hours * 3_600 + minutes * 60;
        return Some(if *sign == b'-' { -offset } else { offset });
    }

    match raw.to_ascii_uppercase().as_str() {
        "UT" | "GMT" => Some(0),
        "EDT" => Some(-4 * 3_600),
        "EST" | "CDT" => Some(-5 * 3_600),
        "CST" | "MDT" => Some(-6 * 3_600),
        "MST" | "PDT" => Some(-7 * 3_600),
        "PST" => Some(-8 * 3_600),
        "J" => None,
        letter if letter.len() == 1 && letter.as_bytes()[0].is_ascii_uppercase() => Some(0),
        _ => None,
    }
}

/// A run of ASCII digits as a number, saturating far past any year; `None`
/// when empty or holding anything else.
fn digits(raw: &str) -> Option<i64> {
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    Some(raw.bytes().fold(0i64, |value, byte| {
        value
            .saturating_mul(10)
            .saturating_add(i64::from(byte - b'0'))
    }))
}

/// One character of a structured header value (RFC 5322 §3.2), with where
/// it stands.
#[derive(Clone, Copy)]
struct Lexeme {
    /// Its byte offset in the value.
    at: usize,
    char: char,
    /// The comment nesting it stands in, an opening parenthesis counting
    /// in the comment it opens and a closing one in the comment it closes.
    depth: usize,
    /// Whether it stands in a quoted-string, its quotes included.
    quoted: bool,
    /// Whether a backslash quotes it, in a comment or a quoted-string.
    escaped: bool,
}

/// The characters of a structured value with where each stands: a comment
/// is text in parentheses, nested, and a quoted-string text in double
/// quotes holding no comment, a backslash in either quoting the next
/// character (RFC 5322 §3.2).
fn lexemes(text: &str) -> impl Iterator<Item = Lexeme> + '_ {
    let mut depth = 0;
    let mut quoted = false;
    let mut escaped = false;

    text.char_indices().map(move |(at, char)| {
        let mut lexeme = Lexeme {
            at,
            char,
            depth,
            quoted,
            escaped: false,
        };
        if escaped {
            escaped = false;
            lexeme.escaped = true;
        } else if char == '\\' && (depth > 0 || quoted) {
            escaped = true;
        } else if char == '(' && !quoted {
            depth += 1;
            lexeme.depth = depth;
        } else if char == ')' && !quoted && depth > 0 {
            depth -= 1;
        } else if char == '"' && depth == 0 {
            quoted = !quoted;
            lexeme.quoted = true;
        }
        lexeme
    })
}

/// A structured value with each comment read as one space (Annex A.1).
fn uncommented(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for lexeme in lexemes(text) {
        if lexeme.depth == 0 {
            out.push(lexeme.char);
        } else if lexeme.char == '(' && lexeme.depth == 1 && !lexeme.escaped {
            out.push(' ');
        }
    }

    out
}

/// Whether a character is whitespace as Annex A.1 reads a header: the
/// space and the tab.
fn is_wsp(char: char) -> bool {
    char == ' ' || char == '\t'
}

/// A text trimmed of spaces and tabs at both ends.
fn trim_wsp(text: &str) -> &str {
    text.trim_matches(is_wsp)
}

/// A text trimmed of spaces and tabs at its end.
fn trim_wsp_end(text: &str) -> &str {
    text.trim_end_matches(is_wsp)
}

/// Splits a MIME entity into its header block and its body: the block
/// ends at the first empty line, a line at CRLF or a bare LF (Annex A.1).
fn split_headers(entity: &[u8]) -> (&[u8], &[u8]) {
    let mut at = 0;
    while let Some(offset) = entity[at..].iter().position(|byte| *byte == b'\n') {
        let end = at + offset;
        if matches!(&entity[at..end], [] | [b'\r']) {
            return (&entity[..at], &entity[end + 1..]);
        }
        at = end + 1;
    }

    (entity, &[])
}

/// A header block, unfolded, as `(lowercased name, value)` pairs, the
/// value trimmed of spaces and tabs.
fn header_fields(block: &[u8]) -> Vec<(String, String)> {
    unfold(block, true)
        .into_iter()
        .take_while(|line| !line.is_empty())
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_lowercase(), trim_wsp(value).to_string()))
        })
        .collect()
}

/// The first occurrence of a header field.
fn field<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header, _)| header == name)
        .map(|(_, value)| value.as_str())
}

/// The fields of a header block as the octets holding each, its folded
/// lines and their line breaks included, under the name [`header_fields`]
/// reads it by. A line holding no colon is none.
fn field_spans(block: &[u8]) -> Vec<(String, Range<usize>)> {
    let mut spans: Vec<Range<usize>> = Vec::new();
    let mut at = 0;
    for line in block.split_inclusive(|byte| *byte == b'\n') {
        let end = at + line.len();
        match (line.first(), spans.last_mut()) {
            (Some(b' ' | b'\t'), Some(last)) => last.end = end,
            _ => spans.push(at..end),
        }
        at = end;
    }

    spans
        .into_iter()
        .filter_map(|span| {
            let line = unfold(&block[span.clone()], true).into_iter().next()?;
            let (name, _) = line.split_once(':')?;
            Some((name.trim().to_lowercase(), span))
        })
        .collect()
}

/// Adds to `cuts` where the message holds a later occurrence of a field
/// an entity is read by ([`ENTITY_FIELDS`]), in the header block of
/// `entity`, which the message holds at `base`, then in the parts under
/// it, walked and bounded as [`has_attachment`] walks them, `read`
/// counting the entities, this one included.
fn restated(
    entity: &[u8],
    base: usize,
    depth: usize,
    read: &mut usize,
    cuts: &mut Vec<Range<usize>>,
) {
    *read += 1;
    let (block, body) = split_headers(entity);
    let mut stated = [false; ENTITY_FIELDS.len()];
    for (name, span) in field_spans(block) {
        let Some(at) = ENTITY_FIELDS.iter().position(|field| *field == name) else {
            continue;
        };
        if core::mem::replace(&mut stated[at], true) {
            cuts.push(base + span.start..base + span.end);
        }
    }

    if depth >= MIME_DEPTH_MAX {
        return;
    }

    let headers = header_fields(block);
    let Some(boundary) = field(&headers, "content-type").and_then(boundary) else {
        return;
    };

    let base = base + entity.len() - body.len();
    for part in part_spans(body, &boundary) {
        if *read >= MIME_ENTITIES_MAX {
            break;
        }
        restated(
            &body[part.clone()],
            base + part.start,
            depth + 1,
            read,
            cuts,
        );
    }
}

/// Whether the entity, nested `depth` multipart levels down, or a part under
/// it, makes the attachment mark ([`makes_mark`]), walking multipart bodies
/// by their boundary down to [`MIME_DEPTH_MAX`] and through
/// [`MIME_ENTITIES_MAX`] entities in document order, `read` counting them,
/// this one included. `protocol` says the entity is an RFC 1847 protocol
/// part of the multipart holding it.
fn has_attachment(
    headers: &[(String, String)],
    body: &[u8],
    depth: usize,
    read: &mut usize,
    protocol: bool,
) -> bool {
    *read += 1;
    let media = mark_media_type(headers);
    // NOTE: the message's own entity, when a multipart, has no section
    // and makes no mark (Annex A.1).
    let multipart_message = depth == 0 && media.starts_with("multipart/");
    if !multipart_message && makes_mark(headers, protocol) {
        return true;
    }

    if depth >= MIME_DEPTH_MAX {
        return false;
    }

    let Some(boundary) = field(headers, "content-type").and_then(boundary) else {
        return false;
    };

    parts(body, &boundary)
        .into_iter()
        .enumerate()
        .any(|(index, part)| {
            let (headers, body) = split_headers(part);
            let protocol = is_protocol_part(&media, index + 1);
            *read < MIME_ENTITIES_MAX
                && has_attachment(&header_fields(headers), body, depth + 1, read, protocol)
        })
}

/// Whether one entity makes the attachment mark (Annex A.1): disposed as an
/// attachment, its `Content-Disposition` type any but `inline` (RFC 2183
/// §2.8 reads an unknown one as `attachment`), or stating no disposition
/// and sent bare as a file, neither `multipart/*` nor `text/*` and named by
/// no `Content-ID`. An RFC 1847 protocol part, `protocol`, never makes it.
fn makes_mark(headers: &[(String, String)], protocol: bool) -> bool {
    if protocol {
        return false;
    }

    let disposition = field(headers, "content-disposition")
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .unwrap_or_default();
    if !disposition.is_empty() {
        return !disposition.eq_ignore_ascii_case("inline");
    }

    let media = mark_media_type(headers);
    !media.starts_with("multipart/")
        && !media.starts_with("text/")
        && field(headers, "content-id").is_none()
}

/// The media type the attachment mark reads (Annex A.1): the `Content-Type`
/// value up to its first `;`, trimmed and lowercased, `text/plain` when the
/// field is absent, under a `multipart/digest` too, or when the value holds
/// not exactly one `/` (RFC 2045 §5.2).
fn mark_media_type(headers: &[(String, String)]) -> String {
    let media = field(headers, "content-type")
        .and_then(|value| value.split(';').next())
        .map(|media| media.trim().to_lowercase())
        .unwrap_or_default();

    if media.matches('/').count() == 1 {
        media
    } else {
        "text/plain".to_string()
    }
}

/// Whether the `index`th child (from 1) of a multipart of media type
/// `parent` is an RFC 1847 protocol part: every child of a
/// `multipart/encrypted`, every child but the first of a
/// `multipart/signed`.
fn is_protocol_part(parent: &str, index: usize) -> bool {
    parent == "multipart/encrypted" || (parent == "multipart/signed" && index > 1)
}

/// The first `UID` of the first `text/calendar` part in document order,
/// its transfer encoding undone and its lines unfolded (Annex A.1), the
/// entity nested `depth` multipart levels down, the walk stopping at
/// [`MIME_DEPTH_MAX`] and after [`MIME_ENTITIES_MAX`] entities, `read`
/// counting them, this one included.
fn invitation(
    headers: &[(String, String)],
    body: &[u8],
    depth: usize,
    read: &mut usize,
) -> Option<String> {
    *read += 1;
    let content_type = field(headers, "content-type").unwrap_or_default();
    let media = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    if media == "text/calendar" {
        // NOTE: the octets decoded before any is read as text, so the
        // streamed walk (attachments::PimdirAttachmentWalk) reads alike.
        let encoding = field(headers, "content-transfer-encoding").map(str::to_lowercase);
        let decoded = match encoding.as_deref().map(str::trim) {
            Some("base64") => base64(body).unwrap_or_default(),
            Some("quoted-printable") => quoted_printable(body),
            _ => body.to_vec(),
        };
        return unfold(&decoded, false).into_iter().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            let name = name.split(';').next().unwrap_or_default();
            name.eq_ignore_ascii_case("UID").then(|| value.to_string())
        });
    }

    if depth >= MIME_DEPTH_MAX {
        return None;
    }

    let boundary = boundary(content_type)?;
    parts(body, &boundary).into_iter().find_map(|part| {
        if *read >= MIME_ENTITIES_MAX {
            return None;
        }
        let (headers, body) = split_headers(part);
        invitation(&header_fields(headers), body, depth + 1, read)
    })
}

/// The boundary of a `multipart/*` content type, `None` for any other.
fn boundary(content_type: &str) -> Option<String> {
    let mut params = content_type.split(';');
    let media = params.next().unwrap_or_default().trim().to_lowercase();
    if !media.starts_with("multipart/") {
        return None;
    }
    params.find_map(|param| {
        let (name, value) = param.split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("boundary")
            .then(|| value.trim().trim_matches('"').to_string())
    })
}

/// The parts of a multipart body, the preamble and epilogue dropped, each
/// with the line break before the delimiter ending it. A body no closing
/// delimiter ends is closed by its end, as one standing there would close
/// it (Annex A.1): what follows its last delimiter line is its last part.
fn parts<'a>(body: &'a [u8], boundary: &str) -> Vec<&'a [u8]> {
    part_spans(body, boundary)
        .into_iter()
        .map(|part| &body[part])
        .collect()
}

/// Where a multipart body holds each of its [`parts`].
fn part_spans(body: &[u8], boundary: &str) -> Vec<Range<usize>> {
    let delimiter = format!("--{boundary}");
    let mut parts = Vec::new();
    let mut start: Option<usize> = None;
    let mut at = 0;

    while at <= body.len() {
        let end = body[at..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(body.len(), |offset| at + offset);
        let line = &body[at..end];
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if let Some(close) = delimits(line, delimiter.as_bytes()) {
            if let Some(from) = start.take() {
                parts.push(from..at);
            }
            if close {
                break;
            }
            start = Some(end + 1);
        }
        at = end + 1;
    }
    // NOTE: a delimiter line ending the body starts a last part holding
    // nothing, as it does before a closing delimiter.
    if let Some(from) = start {
        parts.push(from.min(body.len())..body.len());
    }

    parts
}

/// Whether a line, its line break excluded, is a delimiter of a multipart
/// body, `delimiter` being `--` and its boundary, and whether it closes
/// the body: the delimiter, `--` when it closes, then spaces and tabs
/// alone (RFC 2046 §5.1.1), so a boundary another one begins with never
/// delimits the other.
fn delimits(line: &[u8], delimiter: &[u8]) -> Option<bool> {
    let rest = line.strip_prefix(delimiter)?;
    let (close, rest) = match rest.strip_prefix(b"--") {
        Some(rest) => (true, rest),
        None => (false, rest),
    };

    rest.iter()
        .all(|byte| matches!(byte, b' ' | b'\t'))
        .then_some(close)
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test]
    fn encoded_words_decode_in_both_encodings_and_join_across_whitespace() {
        assert_eq!(decode("=?UTF-8?B?UsOpdW5pb24=?="), "Réunion");
        assert_eq!(decode("=?ISO-8859-1?Q?Z=F6e_Br=FCck?="), "Zöe Brück");
        assert_eq!(decode("=?windows-1252?Q?=80_=93quoted=94?="), "€ “quoted”");
        assert_eq!(decode("=?UTF-8?Q?a?= =?UTF-8?Q?b?="), "ab");
        assert_eq!(decode("plain =?UTF-8?Q?text?= here"), "plain text here");
        assert_eq!(decode("=?bad?="), "=?bad?=");
    }

    #[test]
    fn an_address_list_is_read_in_document_order_with_names_decoded() {
        let list = addresses(
            "\"Bob, Jr.\" <Bob@Example.org>, (note) carol@example.org, =?UTF-8?Q?Z=C3=B6e?= <z@x.y>",
        );
        assert_eq!(
            list,
            vec![
                PimdirAddress {
                    address: "bob@example.org".into(),
                    name: Some("Bob, Jr.".into()),
                },
                PimdirAddress {
                    address: "carol@example.org".into(),
                    name: None,
                },
                PimdirAddress {
                    address: "z@x.y".into(),
                    name: Some("Zöe".into()),
                },
            ]
        );
    }

    #[test]
    fn a_group_is_flattened_and_an_empty_one_yields_nothing() {
        assert_eq!(
            addresses("team: a@x.y, b@x.y;, c@x.y")
                .iter()
                .map(|address| address.address.as_str())
                .collect::<Vec<_>>(),
            vec!["a@x.y", "b@x.y", "c@x.y"]
        );
        assert!(addresses("undisclosed-recipients:;").is_empty());
    }

    #[test]
    fn a_date_is_read_to_utc_with_the_obsolete_zones() {
        assert_eq!(
            instant("Sat, 1 Aug 2026 12:00:00 +0200").as_deref(),
            Some("2026-08-01T10:00:00Z")
        );
        assert_eq!(
            instant("1 Aug 2026 10:00 GMT").as_deref(),
            Some("2026-08-01T10:00:00Z")
        );
        assert_eq!(instant("yesterday"), None);
    }

    #[test]
    fn an_attachment_is_found_under_a_nested_multipart() {
        let body = b"Content-Type: multipart/mixed; boundary=\"outer\"\r\n\r\n--outer\r\nContent-Type: multipart/alternative; boundary=inner\r\n\r\n--inner\r\nContent-Type: text/plain\r\n\r\nhi\r\n--inner--\r\n--outer\r\nContent-Disposition: attachment; filename=a.pdf\r\n\r\n%PDF\r\n--outer--\r\n";
        let (headers, rest) = split_headers(body);
        assert!(has_attachment(
            &header_fields(headers),
            rest,
            0,
            &mut 0,
            false
        ));

        let plain = b"Content-Type: text/plain\r\n\r\nhi\r\n";
        let (headers, rest) = split_headers(plain);
        assert!(!has_attachment(
            &header_fields(headers),
            rest,
            0,
            &mut 0,
            false
        ));
    }

    #[test]
    fn the_mark_without_the_body_reads_the_top_level_content_type() {
        let mixed = b"Content-Type: multipart/mixed; boundary=b\r\nMessage-ID: <m@x>\r\n\r\n--b\r\n\r\nhi\r\n--b--\r\n";
        let mark = |derivation: PimdirDerivation| match derivation.summary {
            Some(PimdirSummary::Mail(mail)) => mail.attachment,
            _ => unreachable!("a message derives a mail summary"),
        };
        assert_eq!(mark(derive_meta(mixed, Some(9), None)), Some(true));
        assert_eq!(mark(derive_meta(mixed, Some(9), Some(false))), Some(false));
        assert_eq!(mark(derive(mixed)), Some(false));
        assert_eq!(
            mark(derive_meta(b"Subject: hi\r\n\r\n", None, None)),
            Some(false)
        );
        assert!(meta_attachment(Some("Multipart/Mixed ; boundary=x")));
        assert!(!meta_attachment(Some("multipart/alternative")));
        assert!(!meta_attachment(None));
    }

    #[test]
    fn a_folded_header_keeps_its_whitespace() {
        let derivation = derive(b"Subject: one\r\n\ttwo   three\r\nMessage-ID: <a@b>\r\n\r\n");
        let PimdirSummary::Mail(mail) = derivation.summary.unwrap() else {
            unreachable!("a message derives a mail summary");
        };
        assert_eq!(mail.subject, "one\ttwo   three");
    }

    #[test]
    fn a_message_without_an_id_keys_on_what_identifies_it() {
        let derivation = derive(
            b"From: a@x.y\r\nSubject: Hi\r\nDate: Sat, 1 Aug 2026 10:00:00 +0000\r\n\r\nbody",
        );
        assert_eq!(
            derivation.link_id.as_str(),
            "alt:Hi|2026-08-01T10:00:00Z|a@x.y"
        );
        assert_eq!(derivation.sort_key.as_str(), "2026-08-01T10:00:00Z");
    }
    #[test]
    fn a_date_outside_annex_a1s_forms_is_unparseable() {
        let date = |raw: &str| instant(raw);
        assert_eq!(date("Sat, 1 Aug 2026 24:00:00 +0000"), None, "24:00");
        assert_eq!(date("Sat, 1 Aug 2026 10:00:00"), None, "no zone");
        assert_eq!(
            date("Sat, 1 Aug 2026 10:00:00 +0000 UTC"),
            None,
            "a word after"
        );
        assert_eq!(date("Sat, 1 Aug 2026 10:00:00 J"), None, "J");
        assert_eq!(date("Sat, 1 Aug 2026 10:00:00 7"), None, "a digit");
        assert_eq!(date("Sat, 1 Aug 2026 10:00:00 +0160"), None, "mm past 59");
        assert_eq!(
            date("Sat, 1 Aug 2026 10:00:00 +02000"),
            None,
            "six characters"
        );
        assert_eq!(
            date("Sat, 1 Aug 2026 1:00:00 +0000"),
            None,
            "one-digit hour"
        );
        assert_eq!(date("Sat, 1 Aug 1899 10:00:00 +0000"), None, "before 1900");
        assert_eq!(date("Sat, 1 Aug 0026 10:00:00 +0000"), None, "four digits");
    }

    #[test]
    fn a_date_reads_comments_colons_and_leap_seconds_as_annex_a1_says() {
        let date = |raw: &str| instant(raw);
        assert_eq!(
            date("Sat, 1 Aug 2026 (sent) 10:00:00 +0000").as_deref(),
            Some("2026-08-01T10:00:00Z"),
            "a comment before the time"
        );
        assert_eq!(
            date("Sat, 1 Aug 2026 10 : 00 : 00 +0000").as_deref(),
            Some("2026-08-01T10:00:00Z"),
            "whitespace around a colon"
        );
        assert_eq!(
            date("Wed, 31 Dec 2026 23:59:60 +0000").as_deref(),
            Some("2026-12-31T23:59:59Z"),
            "60 read as 59, not carried"
        );
        assert_eq!(
            date("Sat, 1 Aug 02026 10:00:00 z").as_deref(),
            Some("2026-08-01T10:00:00Z"),
            "a long year and a lowercase military letter"
        );
    }

    #[test]
    fn a_bracketless_id_holds_one_at_and_no_quote() {
        assert_eq!(message_id("a\"b@x"), None);
        assert_eq!(message_id("a@b@x"), None);
        assert_eq!(message_id("<>"), None, "an empty id");
        assert_eq!(message_id("<a@x"), None, "no > after the <");
        assert_eq!(
            message_ids("a@x junk (c@x) b@y"),
            ["a@x", "b@y"],
            "In-Reply-To word by word, comments removed"
        );
    }

    #[test]
    fn an_encoded_word_is_read_as_annex_a1_states_it() {
        assert_eq!(
            decode("=?iso-8859-1?Q?=80?= =?us-ascii?Q?=93?="),
            "\u{20ac}\u{201c}",
            "iso-8859-1 and us-ascii read as windows-1252"
        );
        assert_eq!(
            decode("=?UTF-8?B?b2s===?="),
            "=?UTF-8?B?b2s===?=",
            "three pads"
        );
        assert_eq!(
            decode("=?UTF-8?B?b2sxx?="),
            "=?UTF-8?B?b2sxx?=",
            "a lone sextet"
        );
        assert_eq!(decode("=?UTF-8?B?b2s?="), "ok", "padding optional");
        assert_eq!(
            decode("=?UTF-8?Q?=C2=A0x=C2=A0?="),
            "\u{a0}x\u{a0}",
            "only spaces and tabs trimmed"
        );
        assert_eq!(
            decode("=?UTF-8?Q?a?= \t=?UTF-8?Q?b?= c =?UTF-8?Q?d?="),
            "ab c d",
            "whitespace between two decoded words alone removed"
        );
    }

    #[test]
    fn a_display_name_reads_comments_as_spaces_and_quotes_anywhere() {
        let name = |raw: &str| addresses(raw).remove(0).name;
        assert_eq!(
            name("Night(ops)Shift <x@y>").as_deref(),
            Some("Night Shift")
        );
        assert_eq!(
            name("Zoe \"the \\\"Z\\\"\" Doe <z@x>").as_deref(),
            Some("Zoe the \"Z\" Doe")
        );
        assert_eq!(name("(only a comment) <z@x>"), None);
        assert_eq!(
            addresses("(a, b) x@y, \"c, d\" <z@w>")
                .iter()
                .map(|address| address.address.as_str())
                .collect::<Vec<_>>(),
            ["x@y", "z@w"],
            "no split inside a comment or a quoted-string"
        );
    }

    #[test]
    fn the_walk_reads_no_entity_past_its_bound() {
        let mut body = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n".to_vec();
        for _ in 1..MIME_ENTITIES_MAX {
            body.extend_from_slice(b"--b\r\n\r\nx\r\n");
        }
        body.extend_from_slice(b"--b\r\nContent-Disposition: attachment\r\n\r\nx\r\n");
        body.extend_from_slice(b"--b\r\nContent-Type: text/calendar\r\n\r\nUID:late\r\n");
        body.extend_from_slice(b"--b--\r\n");

        let Some(PimdirSummary::Mail(mail)) = derive(&body).summary else {
            unreachable!("a message derives a mail summary");
        };
        assert_eq!(mail.attachment, Some(false), "the 10,001st entity unread");
        assert_eq!(mail.invitation, None);
        assert!(attachments::attachments(&body).is_empty());

        // NOTE: a part the end of the message ends is counted as one a
        // delimiter ends, by both walks: read as the 10,000th entity,
        // unread as the 10,001st.
        let cut = |parts: usize| {
            let mut body = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n".to_vec();
            for _ in 0..parts {
                body.extend_from_slice(b"--b\r\n\r\nx\r\n");
            }
            body.extend_from_slice(b"--b\r\nContent-Disposition: attachment\r\n\r\nx\r\n");
            let Some(PimdirSummary::Mail(mail)) = derive(&body).summary else {
                unreachable!("a message derives a mail summary");
            };
            (mail.attachment, attachments::parts(&body).attachment)
        };
        assert_eq!(cut(MIME_ENTITIES_MAX - 2), (Some(true), true));
        assert_eq!(cut(MIME_ENTITIES_MAX - 1), (Some(false), false));
    }

    #[test]
    fn a_boundary_another_begins_with_delimits_only_its_own_body() {
        let body = b"Content-Type: multipart/mixed; boundary=d1\r\n\r\n\
--d1\r\nContent-Type: multipart/mixed; boundary=d10\r\n\r\n\
--d10\r\nContent-Disposition: attachment\r\n\r\nx\r\n--d10--\r\n--d1--\r\n";
        let found = attachments::attachments(body);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].part.as_deref(), Some("1.1"));
        assert_eq!(delimits(b"--d1 \t", b"--d1"), Some(false));
        assert_eq!(delimits(b"--d1--", b"--d1"), Some(true));
        assert_eq!(delimits(b"--d1x", b"--d1"), None);
    }
}

#[cfg(test)]
mod charset_table_tests {
    use super::decode;

    /// A legacy GBK subject of the kind older CN mail servers still emit by
    /// the thousand a week, read in every build, the no_std core's among
    /// them: the tables are no feature (Annex A.1).
    #[test]
    fn a_gbk_subject_is_read_in_every_build() {
        let out = decode("=?gbk?B?1PbBv8r9vt3NrLK9sai45i0yNjEwMjk?=");
        assert_eq!(out, "增量数据同步报告-261029");
    }

    #[test]
    fn the_other_cjk_labels_read() {
        assert_eq!(decode("=?gb2312?B?uqO52A==?="), "海关");
        assert_eq!(decode("=?big5?B?rvzD9g==?="), "海關");
        assert_eq!(decode("=?shift_jis?B?k4yLng==?="), "東京");
    }

    /// A label naming the WHATWG replacement encoding, and one no label
    /// table names, are unknown charsets: the word stays as written.
    #[test]
    fn a_label_naming_no_decoder_keeps_its_word() {
        assert_eq!(
            decode("=?iso-2022-kr?B?GyQpQw5HUTE5Pm4P?="),
            "=?iso-2022-kr?B?GyQpQw5HUTE5Pm4P?="
        );
        assert_eq!(
            decode("=?x-legacy-cjk?B?1PY=?= z"),
            "=?x-legacy-cjk?B?1PY=?= z"
        );
    }

    /// The label holds against a byte-order mark: utf-8 decodes its BOM
    /// as the character it is, never as a sniff that overrides the label.
    #[test]
    fn a_byte_order_mark_decodes_under_its_label() {
        assert_eq!(decode("=?utf-8?B?77u/5aSp5aSp?="), "\u{FEFF}天天");
    }
}
