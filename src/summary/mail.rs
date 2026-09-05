//! # Mail summaries
//!
//! The `message/rfc822` derivation (Annex A.1): the header block decoded
//! (RFC 2047), the addresses canonical (A.6), the `Date` an instant, and
//! the MIME tree walked for an attachment.
//!
//! The decoders are public so a connector building the summary from an
//! IMAP ENVELOPE at the `Meta` tier lands on the same bytes the `Full`
//! tier derives from the body.

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use crate::{
    placement::{PimdirLinkId, PimdirSortKey},
    summary::{PimdirAddress, PimdirDerivation, PimdirSummary, time, unfold},
};

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
    /// Whether a part is an attachment; `None` when the parts were not walked.
    pub attachment: Option<bool>,
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
            Some(id) => PimdirLinkId(id.clone()),
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
pub fn derive(body: &[u8]) -> PimdirDerivation {
    let (headers, rest) = split_headers(body);
    let headers = header_fields(headers);
    let header = |name: &str| field(&headers, name);

    let from = header("from").map(addresses).unwrap_or_default();
    let first = from.first();

    PimdirMailSummary {
        message_id: header("message-id").and_then(strip_angles),
        in_reply_to: header("in-reply-to").map(message_ids).unwrap_or_default(),
        subject: header("subject").map(decode).unwrap_or_default(),
        sender: first.map(|address| address.address.clone()),
        sender_name: first.and_then(|address| address.name.clone()),
        date: header("date").and_then(instant),
        size: Some(body.len() as u64),
        attachment: Some(has_attachment(&headers, rest)),
        from,
        to: header("to").map(addresses).unwrap_or_default(),
        cc: header("cc").map(addresses).unwrap_or_default(),
        bcc: header("bcc").map(addresses).unwrap_or_default(),
    }
    .derivation()
}

/// Decodes a header value: unfolded, RFC 2047 encoded words decoded.
///
/// Adjacent encoded words are joined without the whitespace between them
/// (RFC 2047 §6.2); an unknown charset reads its bytes as UTF-8, lossily.
pub fn decode(raw: &str) -> String {
    let mut out = String::new();
    let mut rest = raw;
    let mut after_word = false;

    while let Some(start) = rest.find("=?") {
        let (text, tail) = rest.split_at(start);
        match encoded_word(tail) {
            Some((decoded, len)) => {
                if !(after_word && text.trim().is_empty()) {
                    out.push_str(text);
                }
                out.push_str(&decoded);
                rest = &tail[len..];
                after_word = true;
            }
            None => {
                out.push_str(text);
                out.push_str("=?");
                rest = &tail[2..];
                after_word = false;
            }
        }
    }
    out.push_str(rest);

    out.trim().to_string()
}

/// One RFC 2047 `encoded-word` at the head of `text`, and its length.
fn encoded_word(text: &str) -> Option<(String, usize)> {
    let inner = text.strip_prefix("=?")?;
    let (charset, rest) = inner.split_once('?')?;
    let (encoding, rest) = rest.split_once('?')?;
    let end = rest.find("?=")?;
    let payload = &rest[..end];

    let bytes = match encoding {
        "B" | "b" => base64(payload)?,
        "Q" | "q" => quoted_printable(payload, true),
        _ => return None,
    };

    let len = 2 + charset.len() + 1 + encoding.len() + 1 + end + 2;
    Some((charset_decode(charset, &bytes), len))
}

/// Bytes under a MIME charset as text: `iso-8859-1` and `us-ascii` by
/// the byte, `windows-1252` by its table, and every other charset as
/// UTF-8, replacing what does not decode (Annex A.0). The crate carries
/// no charset tables beyond these, so another 8-bit charset reads lossily.
fn charset_decode(charset: &str, bytes: &[u8]) -> String {
    let charset = charset.split('*').next().unwrap_or_default().to_lowercase();
    match charset.as_str() {
        "iso-8859-1" | "latin1" | "us-ascii" | "ascii" => {
            bytes.iter().map(|byte| char::from(*byte)).collect()
        }
        "windows-1252" | "cp1252" => bytes.iter().map(|byte| cp1252(*byte)).collect(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
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

/// Decodes RFC 2045 quoted-printable, with the `_` of RFC 2047 §4.2 when
/// `header` is set.
fn quoted_printable(text: &str, header: bool) -> Vec<u8> {
    let bytes = text.as_bytes();
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
            b'_' if header => {
                out.push(b' ');
                at += 1;
            }
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
fn base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer: u32 = 0;
    let mut bits = 0;

    for byte in text.bytes() {
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

/// The addresses of an address-list header, in document order.
///
/// Groups are flattened, comments dropped, display names decoded and
/// unquoted, and every addr-spec made canonical (Annex A.6).
pub fn addresses(raw: &str) -> Vec<PimdirAddress> {
    mailboxes(raw)
        .iter()
        .filter_map(|mailbox| {
            let mailbox = mailbox.trim();
            let (name, address) = match (mailbox.find('<'), mailbox.rfind('>')) {
                (Some(open), Some(close)) if open < close => {
                    (Some(&mailbox[..open]), &mailbox[open + 1..close])
                }
                _ => (None, mailbox),
            };
            let address = PimdirAddress::canonical(address);
            if address.is_empty() {
                return None;
            }

            let name = name
                .map(|name| decode(&unquote(name.trim())))
                .filter(|name| !name.is_empty());
            Some(PimdirAddress { address, name })
        })
        .collect()
}

/// Splits an address-list into its mailboxes: at the commas outside
/// quotes, comments and angle brackets, with a group's name and its
/// closing semicolon removed.
fn mailboxes(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut comment = 0;
    let mut angled = false;
    let mut escaped = false;

    for char in raw.chars() {
        if escaped {
            current.push(char);
            escaped = false;
            continue;
        }
        match char {
            '\\' if quoted => {
                current.push(char);
                escaped = true;
            }
            '"' if comment == 0 => {
                quoted = !quoted;
                current.push(char);
            }
            '(' if !quoted => comment += 1,
            ')' if !quoted && comment > 0 => comment -= 1,
            _ if comment > 0 => {}
            '<' if !quoted => {
                angled = true;
                current.push(char);
            }
            '>' if !quoted => {
                angled = false;
                current.push(char);
            }
            ':' if !quoted && !angled && !current.contains('@') => current.clear(),
            ',' | ';' if !quoted && !angled => {
                out.push(core::mem::take(&mut current));
            }
            _ => current.push(char),
        }
    }
    out.push(current);

    out.into_iter()
        .filter(|mailbox| !mailbox.trim().is_empty())
        .collect()
}

/// A display name with its surrounding quotes and their escapes removed.
fn unquote(name: &str) -> String {
    let Some(inner) = name
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return name.to_string();
    };

    let mut out = String::with_capacity(inner.len());
    let mut escaped = false;
    for char in inner.chars() {
        match (escaped, char) {
            (false, '\\') => escaped = true,
            _ => {
                out.push(char);
                escaped = false;
            }
        }
    }

    out
}

/// The bare ids of an `In-Reply-To` or `References` value (RFC 5322
/// §3.6.4, `1*msg-id`), read off their angle brackets when they carry
/// any and off whitespace otherwise.
pub fn message_ids(raw: &str) -> Vec<String> {
    if raw.contains('<') {
        return raw
            .split('<')
            .filter_map(|rest| rest.split_once('>'))
            .filter_map(|(id, _)| strip_angles(id))
            .collect();
    }

    raw.split_whitespace().filter_map(strip_angles).collect()
}

/// Strips a `msg-id`'s angle brackets; an empty result is no id at all.
fn strip_angles(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let inner = trimmed
        .strip_prefix('<')
        .and_then(|rest| rest.strip_suffix('>'))
        .unwrap_or(trimmed)
        .trim();

    (!inner.is_empty()).then(|| inner.to_string())
}

/// An RFC 5322 `date-time` as the RFC 3339 instant in UTC the summary
/// stores, or `None` when it does not parse: a date is never invented.
///
/// `[day-of-week ","] day month year hour ":" minute [":" second] zone`,
/// with the obsolete alphabetic zones RFC 5322 §4.3 keeps.
pub fn instant(raw: &str) -> Option<String> {
    let raw = raw.split(&['(', ';'][..]).next().unwrap_or_default();
    let rest = match raw.split_once(',') {
        Some((_, rest)) => rest,
        None => raw,
    };

    let mut fields = rest.split_whitespace();
    let day: u32 = fields.next()?.parse().ok()?;
    let month = month(fields.next()?)?;
    let year = year(fields.next()?)?;

    let mut clock = fields.next()?.split(':');
    let hour: u32 = clock.next()?.parse().ok()?;
    let minute: u32 = clock.next()?.parse().ok()?;
    let second: u32 = clock.next().unwrap_or("0").parse().ok()?;
    if day == 0 || day > 31 || hour > 24 || minute > 59 || second > 60 {
        return None;
    }

    let offset = zone(fields.next().unwrap_or("-0000"))?;
    Some(time::rfc3339(
        time::unix(year, month, day, hour, minute, second) - offset,
    ))
}

/// The month a `date-time`'s three-letter name stands for.
fn month(name: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];

    let name = name.to_lowercase();
    MONTHS
        .iter()
        .position(|month| *month == name)
        .map(|index| index as u32 + 1)
}

/// A `date-time`'s year, with the two- and three-digit forms of RFC 5322 §4.3.
fn year(raw: &str) -> Option<i32> {
    let year: i32 = raw.parse().ok()?;
    match raw.len() {
        2 if year < 50 => Some(2000 + year),
        2 => Some(1900 + year),
        3 => Some(1900 + year),
        _ => Some(year),
    }
}

/// A `date-time`'s zone, as the seconds to subtract to reach UTC.
///
/// `-0000` states an unknown zone rather than UTC, and names the same
/// instant, which is all a key needs; so does every other single letter.
fn zone(raw: &str) -> Option<i64> {
    let bytes = raw.as_bytes();
    if matches!(bytes.first(), Some(b'+' | b'-')) && bytes.len() >= 5 {
        let hours = time::digits(bytes, 1, 2)? as i64;
        let minutes = time::digits(bytes, 3, 2)? as i64;
        let offset = hours * 3_600 + minutes * 60;
        return Some(if bytes[0] == b'-' { -offset } else { offset });
    }

    match raw.to_uppercase().as_str() {
        "UT" | "GMT" | "Z" => Some(0),
        "EDT" => Some(-4 * 3_600),
        "EST" | "CDT" => Some(-5 * 3_600),
        "CST" | "MDT" => Some(-6 * 3_600),
        "MST" | "PDT" => Some(-7 * 3_600),
        "PST" => Some(-8 * 3_600),
        name if name.len() == 1 => Some(0),
        _ => None,
    }
}

/// Splits a MIME entity into its header block and its body.
fn split_headers(entity: &[u8]) -> (&[u8], &[u8]) {
    for (at, window) in entity.windows(2).enumerate() {
        if window == b"\n\n" {
            return (&entity[..at + 1], &entity[at + 2..]);
        }
        if window == b"\r\n" && entity.get(at + 2..at + 4) == Some(b"\r\n") {
            return (&entity[..at + 2], &entity[at + 4..]);
        }
    }

    (entity, &[])
}

/// A header block, unfolded, as `(lowercased name, value)` pairs.
fn header_fields(block: &[u8]) -> Vec<(String, String)> {
    unfold(block, true)
        .into_iter()
        .take_while(|line| !line.is_empty())
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_lowercase(), value.trim().to_string()))
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

/// Whether the entity, or a part under it, carries `Content-Disposition:
/// attachment`, walking multipart bodies by their boundary.
fn has_attachment(headers: &[(String, String)], body: &[u8]) -> bool {
    if field(headers, "content-disposition")
        .and_then(|value| value.split(';').next())
        .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("attachment"))
    {
        return true;
    }

    let Some(content_type) = field(headers, "content-type") else {
        return false;
    };
    let mut params = content_type.split(';');
    let media = params.next().unwrap_or_default().trim().to_lowercase();
    if !media.starts_with("multipart/") {
        return false;
    }
    let Some(boundary) = params.find_map(|param| {
        let (name, value) = param.split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("boundary")
            .then(|| value.trim().trim_matches('"').to_string())
    }) else {
        return false;
    };

    parts(body, &boundary).into_iter().any(|part| {
        let (headers, body) = split_headers(part);
        has_attachment(&header_fields(headers), body)
    })
}

/// The parts of a multipart body, the preamble and epilogue dropped.
fn parts<'a>(body: &'a [u8], boundary: &str) -> Vec<&'a [u8]> {
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
        if let Some(rest) = line.strip_prefix(delimiter.as_bytes()) {
            if let Some(from) = start {
                parts.push(&body[from..at]);
            }
            if rest.starts_with(b"--") {
                break;
            }
            start = Some(end + 1);
        }
        at = end + 1;
    }

    parts
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
        assert!(has_attachment(&header_fields(headers), rest));

        let plain = b"Content-Type: text/plain\r\n\r\nhi\r\n";
        let (headers, rest) = split_headers(plain);
        assert!(!has_attachment(&header_fields(headers), rest));
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
}
