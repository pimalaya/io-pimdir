//! What a writer derives from an item's bytes before handing them to the
//! store: its `link_id`, its `meta` summary and its `sort_key` (spec
//! Annex A).
//!
//! The store never parses `meta` (spec §13) and this module does not
//! change that: it is a library the writer calls before the bytes reach
//! the store, the way [`hash`](crate::hash) is one for naming them. What
//! it removes is the agreement two writers of one collection had to keep
//! by hand. Annex A is informative, so nothing enforces it, and
//! consumers implementing it separately diverge: one writes a whole
//! calendar summary and a resolved key, another an empty key for every
//! item, and the first then reads the second's calendar with no row to
//! render and no ordering.
//!
//! The fallback ids are fixed here for the same reason, and it is the
//! sharper one: two writers disagreeing about the id of a message with
//! no `Message-ID` link it twice and store one body twice, the
//! object-hash bug on the identity axis.
//!
//! Each kind is read by a small scanner rather than by a parser. A body
//! crosses this crate byte for byte, and the fields a summary holds are
//! a shallow read of a handful of properties. The `no_std` core is also
//! why: the parsers a frontend renders with (vcard-rs, ical-rs,
//! mail-parser) belong where the rendering happens.

pub mod calendar;
pub mod card;
pub mod mail;
mod time;

use alloc::string::String;

use io_replica::placement::{ReplicaLinkId, ReplicaMeta, ReplicaSortKey};

/// Everything a writer derives from one item's bytes.
///
/// Kept together because all three come from one read and are written
/// together: a mutation refreshing the body without the key leaves the
/// item sorted where its old start put it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirDerivation {
    /// The item's cross-source identity (spec §9). Never absent: content
    /// carrying no usable id falls back to one derived from what it does
    /// carry, refusing the item being a way to lose it.
    pub link_id: ReplicaLinkId,
    /// The `v: 1` summary blob for the kind (spec Annex A).
    pub meta: ReplicaMeta,
    /// The kind's ordering key (spec §9.3), empty when nothing orderable
    /// was found.
    pub sort_key: ReplicaSortKey,
}

/// Derives from a body of the given kind, or `None` when this crate has no
/// conventions for that media type.
///
/// The kind is the collection's declared `kind` (spec §14), matched on
/// the bare media type: `text/vcard; charset=utf-8` reads as
/// `text/vcard`.
pub fn derive(kind: &str, body: &[u8]) -> Option<PimdirDerivation> {
    let kind = kind.split(';').next().unwrap_or_default().trim();

    match kind {
        "message/rfc822" => Some(mail::derive(body)),
        "text/vcard" => Some(card::derive(body)),
        "text/calendar" => Some(calendar::derive(body)),
        _ => None,
    }
}

/// The FNV-1a 64 digest of a body, as the sixteen lowercase hexadecimal
/// digits a fallback id carries.
///
/// A fallback needs to be stable across writers and cheap, not
/// collision-resistant: the body it names is stored under the store's own
/// hash (spec §5) either way, and this only has to keep two writers from
/// linking one item twice.
fn fnv1a64(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }

    alloc::format!("{hash:016x}")
}

/// Unfolds a folded body into its logical lines (RFC 5322 §2.2.3, RFC
/// 5545 §3.1, RFC 6350 §3.2): a line beginning with a space or a tab
/// continues the one before it.
///
/// Invalid UTF-8 is replaced rather than refused: a summary carrying a
/// replacement character beats no summary at all, and the body itself is
/// stored untouched either way.
fn unfold(body: &[u8], keep_leading_space: bool) -> alloc::vec::Vec<String> {
    let text = alloc::string::String::from_utf8_lossy(body);
    let mut lines: alloc::vec::Vec<String> = alloc::vec::Vec::new();

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
