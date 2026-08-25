//! The `text/vcard` conventions (spec Annex A.2).
//!
//! A card has one derivation: a CardDAV `sync-collection` REPORT returns hrefs
//! and ETags but no `UID`, so a card resolves at `Full` only and there is no
//! second reading to keep in agreement.
//!
//! The values are carried verbatim, escapes and all, because `fn` is what a
//! reader displays and the card itself is stored untouched beside it. Only the
//! sort key is normalised.

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use io_replica::placement::{ReplicaLinkId, ReplicaMeta, ReplicaSortKey};
use serde::{Deserialize, Serialize};

use crate::conventions::{PimdirDerivation, fnv1a64, unfold};

/// The `text/vcard` summary (spec Annex A.2), `v: 1`.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PimdirCardMeta {
    /// The convention version, `1` today.
    pub v: u8,
    /// The card's `UID`, verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
    /// The `FN` display name, verbatim: whitespace and case included, since
    /// this is what a reader shows.
    #[serde(default, rename = "fn")]
    pub fn_: String,
    /// Every `EMAIL` property, in document order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emails: Vec<String>,
    /// The raw card octets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// Derives a card's link id, summary and sort key.
///
/// The link id is the `UID`. A card carrying none falls back to `hash:` over
/// the body, which is the only thing left that identifies it: a card has no
/// date and no sender to stand in.
pub fn derive(body: &[u8]) -> PimdirDerivation {
    let lines = unfold(body, false);
    let uid = property(&lines, "UID");
    let display_name = property(&lines, "FN").unwrap_or_default();

    let link_id = match &uid {
        Some(uid) => uid.clone(),
        None => format!("hash:{}", fnv1a64(body)),
    };

    let meta = PimdirCardMeta {
        v: 1,
        uid,
        fn_: display_name.clone(),
        emails: properties(&lines, "EMAIL"),
        size: Some(body.len() as u64),
    };

    PimdirDerivation {
        link_id: ReplicaLinkId(link_id),
        meta: ReplicaMeta(serde_json::to_string(&meta).unwrap_or_default()),
        sort_key: ReplicaSortKey(sort_key(&display_name)),
    }
}

/// The display name normalised for ordering: lowercased, then trimmed.
///
/// Annex A.2 asks for the Unicode **simple** lowercase mapping, which Rust does
/// not expose: [`str::to_lowercase`] is the full mapping, and the two differ on
/// exactly two points (the Greek final sigma, and `İ` expanding to two chars).
/// Neither appears in a display name often enough to prefer a hand-rolled table
/// to the standard library's, and the format's own vectors stay ASCII for the
/// same reason.
fn sort_key(display_name: &str) -> String {
    display_name.to_lowercase().trim().to_string()
}

/// The value of the first occurrence of a property.
fn property(lines: &[String], name: &str) -> Option<String> {
    lines.iter().find_map(|line| value_of(line, name))
}

/// The values of every occurrence of a property, in document order.
fn properties(lines: &[String], name: &str) -> Vec<String> {
    lines
        .iter()
        .filter_map(|line| value_of(line, name))
        .collect()
}

/// One content line's value, when it carries the named property.
///
/// A line is `[group "."] name *(";" param) ":" value` (RFC 6350 §3.3), and the
/// group is dropped: `item1.EMAIL` is an `EMAIL`.
fn value_of(line: &str, name: &str) -> Option<String> {
    let (head, value) = line.split_once(':')?;
    let head = head.split(';').next().unwrap_or_default();
    let property = head.rsplit('.').next().unwrap_or_default();

    property
        .trim()
        .eq_ignore_ascii_case(name)
        .then(|| value.to_string())
}
