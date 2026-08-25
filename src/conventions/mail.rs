//! The `message/rfc822` conventions (spec Annex A.1).
//!
//! A message is read as its header block alone: every field a summary carries
//! is a header, and the body is never touched. `Date:` is normalised to UTC
//! here and only here, because two writers either side of the sender would
//! otherwise record the same message differently and a reader comparing two
//! accounts would see two dates.

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use io_replica::placement::{ReplicaLinkId, ReplicaMeta, ReplicaSortKey};
use serde::{Deserialize, Serialize};

use crate::conventions::{PimdirDerivation, time, unfold};

/// The `message/rfc822` summary (spec Annex A.1), `v: 1`.
///
/// Every optional field absent means unknown. Flags are not here: they are the
/// item's own (spec §13).
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PimdirMailMeta {
    /// The convention version, `1` today.
    pub v: u8,
    /// The bare `Message-ID`, angle brackets stripped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    /// The `In-Reply-To` ids, bare and stripped the same way, so a reply and
    /// its parent compare byte for byte. An array because RFC 5322 §3.6.4
    /// makes the field `1*msg-id`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub in_reply_to: Vec<String>,
    /// The `Subject`. Required, and may be empty.
    #[serde(default)]
    pub subject: String,
    /// The first sender's bare `addr-spec`, the display name stripped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The first recipient's bare `addr-spec`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// The `Date`, normalised to RFC 3339 in UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// The raw message octets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// Derives a message's link id, summary and sort key.
///
/// The link id is the bare `Message-ID`. A message carrying none falls back to
/// `alt:` over the three fields that identify it in practice — subject, date
/// and sender — rather than to the body's hash, so a message re-fetched at a
/// different detail tier still links to the item it already has.
pub fn derive(body: &[u8]) -> PimdirDerivation {
    let headers = headers(body);
    let message_id = header(&headers, "message-id").and_then(strip_angles);
    let date = header(&headers, "date").and_then(rfc3339_from_rfc5322);
    let subject = header(&headers, "subject").unwrap_or_default().to_string();
    let from = header(&headers, "from").and_then(first_address);
    let to = header(&headers, "to").and_then(first_address);

    let link_id = match &message_id {
        Some(id) => id.clone(),
        None => format!(
            "alt:{subject}|{}|{}",
            date.as_deref().unwrap_or_default(),
            from.as_deref().unwrap_or_default(),
        ),
    };

    let meta = PimdirMailMeta {
        v: 1,
        message_id,
        in_reply_to: header(&headers, "in-reply-to")
            .map(message_ids)
            .unwrap_or_default(),
        subject,
        from,
        to,
        date: date.clone(),
        size: Some(body.len() as u64),
    };

    PimdirDerivation {
        link_id: ReplicaLinkId(link_id),
        meta: ReplicaMeta(serde_json::to_string(&meta).unwrap_or_default()),
        sort_key: ReplicaSortKey(date.unwrap_or_default()),
    }
}

/// The message's header block, unfolded, as `(lowercased name, value)`.
fn headers(body: &[u8]) -> Vec<(String, String)> {
    unfold(body, true)
        .into_iter()
        .take_while(|line| !line.is_empty())
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_lowercase(), value.trim().to_string()))
        })
        .collect()
}

/// The first occurrence of a header, which is the one RFC 5322 §3.6 allows for
/// every field a summary reads.
fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header, _)| header == name)
        .map(|(_, value)| value.as_str())
}

/// Strips a `msg-id`'s angle brackets, so every writer's id compares byte for
/// byte. An empty result is no id at all.
fn strip_angles(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let inner = trimmed
        .strip_prefix('<')
        .and_then(|rest| rest.strip_suffix('>'))
        .unwrap_or(trimmed)
        .trim();

    (!inner.is_empty()).then(|| inner.to_string())
}

/// Splits an `In-Reply-To` value into its bare ids (RFC 5322 §3.6.4, `1*msg-id`).
///
/// The ids are read off the angle brackets that delimit them; a value carrying
/// none is split on whitespace instead, since that is what a writer that
/// stripped them leaves behind.
fn message_ids(raw: &str) -> Vec<String> {
    if raw.contains('<') {
        return raw
            .split('<')
            .filter_map(|rest| rest.split_once('>'))
            .filter_map(|(id, _)| strip_angles(id))
            .collect();
    }

    raw.split_whitespace().filter_map(strip_angles).collect()
}

/// The bare `addr-spec` of the first address in an address list, the display
/// name stripped: `Alice Example <alice@example.org>` reads `alice@example.org`.
fn first_address(raw: &str) -> Option<String> {
    let mut quoted = false;
    let mut angled = false;
    let mut first = raw;

    for (at, char) in raw.char_indices() {
        match char {
            '"' => quoted = !quoted,
            '<' if !quoted => angled = true,
            '>' if !quoted => angled = false,
            ',' if !quoted && !angled => {
                first = &raw[..at];
                break;
            }
            _ => {}
        }
    }

    let address = match (first.find('<'), first.find('>')) {
        (Some(open), Some(close)) if open < close => &first[open + 1..close],
        _ => first,
    };

    let address = address.trim();
    (!address.is_empty()).then(|| address.to_string())
}

/// Reads an RFC 5322 `date-time` as the RFC 3339 instant in UTC the convention
/// stores, or `None` when it does not parse.
///
/// `[day-of-week ","] day month year hour ":" minute [":" second] zone`, with
/// the obsolete alphabetic zones RFC 5322 §4.3 still requires a reader to
/// accept. Nothing is guessed: a date this cannot read is absent rather than
/// invented, and the item lands at the end of a descending listing.
fn rfc3339_from_rfc5322(raw: &str) -> Option<String> {
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

/// A `date-time`'s year, with the two- and three-digit forms RFC 5322 §4.3
/// keeps for the messages that carry them.
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
/// `+hhmm` and `-hhmm`, plus the obsolete names. `-0000` states an unknown
/// zone rather than UTC, but the instant it names is the same one, which is
/// all a key needs.
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
        // NOTE: RFC 5322 §4.3 makes every other single letter, `J` included,
        // mean an unknown zone, which is `-0000` and therefore UTC's instant.
        name if name.len() == 1 => Some(0),
        _ => None,
    }
}
