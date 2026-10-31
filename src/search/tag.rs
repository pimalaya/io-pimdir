//! # Tags
//!
//! Tagging from a query (SEARCH §10): the hits of a query, every live
//! placement of each, and the flag set each would carry once `add` is
//! added and `remove` removed, staged as one `set-flags` action per
//! placement through the queue (STORAGE §15), as a producer. A placement
//! the queue holds a `set-flags` of is planned from the flags that one
//! sets, so tags staged in a row compose.
//!
//! The plan is data: the index's `tag` answers it without writing, so a
//! caller can show it before it writes, and its `stage` appends it
//! (`client::search`, feature `client`). Applying it is the owner's: a
//! drain, or the next sync.

use alloc::{
    collections::BTreeSet,
    string::{String, ToString},
    vec::Vec,
};
use core::fmt;

use crate::{
    placement::{PimdirFlags, PimdirLinkId},
    search::coverage::PimdirSearchCoverage,
};

/// What a tag stages (SEARCH §10): every hit of its query, each placement
/// with the flag set it would carry.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirSearchTagPlan {
    /// The flags added, `is:` spellings mapped to their flags.
    pub add: BTreeSet<String>,
    /// The flags removed, mapped likewise.
    pub remove: BTreeSet<String>,
    /// Every hit of the query, in date order.
    pub hits: Vec<PimdirSearchTagHit>,
    /// What the query could not see (SEARCH §8.2): a tag reaches only the
    /// hits the index answers.
    pub coverage: PimdirSearchCoverage,
}

/// One hit a tag reaches, `(account, seq)`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirSearchTagHit {
    /// The account, `None` in a single-account store.
    pub account: Option<String>,
    /// The public id.
    pub seq: i64,
    /// The kind, as a search hit names it.
    pub kind: String,
    /// Its subject, name or summary.
    pub title: String,
    /// Its live placements in the account.
    pub placements: Vec<PimdirSearchTagPlacement>,
}

/// One placement of a hit and the flag set a tag gives it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirSearchTagPlacement {
    /// The collection.
    pub collection: String,
    /// The item's key.
    pub link_id: PimdirLinkId,
    /// Its flags now, as the last pending `set-flags` of it leaves them
    /// when the queue holds one.
    pub flags: PimdirFlags,
    /// The flag set the tag stages for it; `None` when its flags were
    /// never read, so no absolute set can be staged without losing them.
    pub next: Option<BTreeSet<String>>,
    /// The source its action is staged as, or why there is none.
    pub source: PimdirSearchTagSource,
    /// Whether its source carries the change to its server by its
    /// declared capabilities (Annex B.1): only mail has flags there, and
    /// only a source declaring the flags changed (or declaring nothing)
    /// pushes them. False keeps the tag in the store alone.
    pub syncs: bool,
    /// The capabilities the change needs that its source declares no
    /// support for (Annex B.1): not empty, the placement is not staged,
    /// since the source would refuse it.
    pub missing: Vec<String>,
}

/// The source a placement's action is staged as (STORAGE §15): one that
/// syncs its collection, never another, so it is pushed where the
/// collection lives.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirSearchTagSource {
    /// The source asked for, which syncs the collection, else the only
    /// one that does.
    Resolved(String),
    /// Several sources sync the collection and none was asked for.
    Ambiguous(Vec<String>),
    /// The source asked for does not sync the collection, or no source
    /// does: the sources that do, possibly none.
    Unsynced(Vec<String>),
}

impl PimdirSearchTagPlacement {
    /// Whether the tag changes its flags, so an action is staged for it
    /// when its source is resolved and supports the change.
    pub fn changes(&self) -> bool {
        self.next
            .as_ref()
            .is_some_and(|next| Some(next) != self.flags.known())
    }
}

/// One `set-flags` action a tag appended to the queue.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirSearchTagAction {
    /// The collection it is anchored on.
    pub collection: String,
    /// The item's public id.
    pub seq: i64,
    /// The source it is staged as: the one a drain applies it as.
    pub source: String,
    /// The flag set it sets.
    pub flags: BTreeSet<String>,
    /// The queue row's id.
    pub action: i64,
}

/// A flag change a tag refuses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirSearchTagError {
    /// Nothing to add or remove.
    Empty,
    /// One flag both added and removed.
    Both(String),
    /// A value that names no flag: empty, or an `is:` value that is the
    /// absence of a flag (`unread`) or no flag at all (`encrypted`).
    NotAFlag(String),
}

impl fmt::Display for PimdirSearchTagError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "A tag needs a flag to add or to remove"),
            Self::Both(flag) => write!(f, "Flag {flag} is both added and removed"),
            Self::NotAFlag(value) if value == "unread" => {
                write!(f, "unread is the absence of \\Seen: remove read instead")
            }
            Self::NotAFlag(value) => write!(f, "{value:?} names no flag"),
        }
    }
}

impl core::error::Error for PimdirSearchTagError {}

/// The flags of a tag's values (SEARCH §8): an `is:` spelling naming a
/// flag's presence read as that flag, any other value as written.
pub fn flags(values: &[String]) -> Result<BTreeSet<String>, PimdirSearchTagError> {
    values
        .iter()
        .map(|value| match value.as_str() {
            "read" => Ok("\\Seen".to_string()),
            "flagged" => Ok("\\Flagged".to_string()),
            "answered" => Ok("\\Answered".to_string()),
            "draft" => Ok("\\Draft".to_string()),
            "" | "unread" | "encrypted" => Err(PimdirSearchTagError::NotAFlag(value.clone())),
            flag => Ok(flag.to_string()),
        })
        .collect()
}
