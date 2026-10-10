//! # Reference
//!
//! What one item says about another, whatever their kinds (STORAGE
//! §14.2): a message and the file it attaches, an invitation and its
//! event, two things a person linked. A reference names each end by its
//! kind and link id, never by a collection, so it survives a move, and
//! goes with the last row of either end. It is a fact recorded once and
//! never recomputed; nothing requires one.

use alloc::string::String;

use crate::placement::PimdirLinkId;

/// One end of a reference: a collection kind and a link id.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirEndpoint {
    /// The kind of the collections holding the item, as
    /// `collections.kind` holds it (`message/rfc822`, `text/vcard`).
    pub kind: String,
    /// The item's key, the same in every collection holding it.
    pub link_id: PimdirLinkId,
}

/// What a reference says (STORAGE §14.2).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirReferenceRole {
    /// A message and a file it attaches.
    Attachment,
    /// An invitation and its event.
    Invitation,
    /// A message and the contact who sent it.
    Sender,
    /// Two items a person or an application relates.
    Related,
    /// An application's own role: the whole name, `x-` included, which
    /// the store refuses otherwise.
    Application(String),
}

impl PimdirReferenceRole {
    /// The role as its column spelling.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Attachment => "attachment",
            Self::Invitation => "invitation",
            Self::Sender => "sender",
            Self::Related => "related",
            Self::Application(name) => name,
        }
    }

    /// The inverse of [`as_str`](Self::as_str); a name pimdir does not
    /// give is an application's.
    pub fn parse(value: &str) -> Self {
        match value {
            "attachment" => Self::Attachment,
            "invitation" => Self::Invitation,
            "sender" => Self::Sender,
            "related" => Self::Related,
            name => Self::Application(name.into()),
        }
    }
}

/// Who recorded a reference (STORAGE §14.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PimdirReferenceOrigin {
    /// A writer's rule, which may record it again while it matches.
    Auto,
    /// A person, whose reference takes over a rule's.
    User,
}

impl PimdirReferenceOrigin {
    /// The origin as its column spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::User => "user",
        }
    }

    /// The inverse of [`as_str`](Self::as_str); an unknown spelling is
    /// `Auto`.
    pub fn parse(value: &str) -> Self {
        match value {
            "user" => Self::User,
            _ => Self::Auto,
        }
    }
}

/// A reference between two items, as the store records it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirReference {
    /// The referring item.
    pub from: PimdirEndpoint,
    /// The referred item.
    pub to: PimdirEndpoint,
    /// What the reference says.
    pub role: PimdirReferenceRole,
    /// Who recorded it.
    pub origin: PimdirReferenceOrigin,
    /// When it was recorded, an RFC 3339 instant stamped by the store.
    pub created_at: String,
}
