//! # Collection
//!
//! A mailbox, address book or calendar: the id every verb is scoped to,
//! and the opaque sync token round-tripped between the two seams.

use alloc::vec::Vec;

crate::pimdir_id! {
    /// The account-scoped identity of a collection.
    PimdirCollectionId, Ord, PartialOrd, Hash,
}

/// An opaque per-collection sync token.
///
/// A QRESYNC pack, a JMAP state string or a WebDAV sync-token, never
/// inspected by the engine, only round-tripped between the two seams.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirCheckpoint(pub Vec<u8>);

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use crate::collection::PimdirCollectionId;

    #[test]
    fn id_converts_from_owned_and_borrowed_strings() {
        let owned = PimdirCollectionId::from(String::from("inbox"));
        let borrowed = PimdirCollectionId::from("inbox");
        assert_eq!(owned, borrowed);
        assert_eq!(owned.as_str(), "inbox");
    }
}
