//! # File summaries
//!
//! The `application/octet-stream` summary (Annex A.7): a file's bytes
//! say nothing about it, so its name, media type, size and, for an
//! attachment, its part are what its source or its writer states. The
//! key is a name the store gives: `part:` for an attachment, `file:` for
//! a file created in the store.

use alloc::{
    format,
    string::{String, ToString},
};

use crate::{
    placement::{PimdirLinkId, PimdirSortKey},
    summary::{PimdirDerivation, PimdirSummary},
};

/// The kind of every file collection (STORAGE §14.3).
pub const KIND: &str = "application/octet-stream";

/// The `file_summary` row of one placement of a file.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirFileSummary {
    /// The file's name in this collection; empty when none.
    pub name: String,
    /// The type and subtype, lowercased, parameters dropped.
    pub media_type: Option<String>,
    /// The octets: the body's when held, else as stated.
    pub size: Option<u64>,
    /// An attachment's IMAP section in its message (`2`, `1.2`).
    pub part: Option<String>,
}

impl PimdirFileSummary {
    /// The name lowercased by the Unicode simple mapping, then trimmed;
    /// read ascending, as a card's name is.
    pub fn sort_key(&self) -> PimdirSortKey {
        PimdirSortKey(self.name.to_lowercase().trim().to_string())
    }

    /// The derivation filing this summary under `link_id`, the bytes
    /// stating no key of their own.
    pub fn derivation(self, link_id: PimdirLinkId) -> PimdirDerivation {
        PimdirDerivation {
            link_id,
            sort_key: self.sort_key(),
            summary: Some(PimdirSummary::File(self)),
        }
    }
}

/// The key of an attachment (Annex A.7): `part:`, the message's link id,
/// `#` and the part's section, so every writer walking one message names
/// its files alike; `None` for a message under a writer-derived key,
/// which names no identity and gets no stand-in (STORAGE §14.3).
pub fn part_key(message: &PimdirLinkId, section: &str) -> Option<PimdirLinkId> {
    (!message.is_derived()).then(|| PimdirLinkId(format!("part:{}#{section}", message.as_str())))
}
