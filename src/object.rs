//! # Object
//!
//! The content-addressed item body, stored once and shared by every
//! placement pointing at it.
//!
//! One of the two identity axes, next to [`crate::placement`]. An edit
//! of mutable content is a new object, the old one dereferenced. Many
//! placements sharing one object is the dedup and unified-view mechanism.

crate::pimdir_id! {
    /// The content hash naming an object in the object store.
    ///
    /// Collision-resistant (a truncated SHA-256, say). Size is only a
    /// cheap pre-check: equal size does not imply equal bytes.
    PimdirHash, Ord, PartialOrd, Hash,
}

/// A stored object: its content hash and byte size.
///
/// The bytes live out of band at `blobdir/<hash>`, refcounted so copy,
/// move and undelete are reference edits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirObject {
    /// The content hash naming the bytes.
    pub hash: PimdirHash,
    /// The byte size of the content.
    pub size: usize,
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use crate::object::PimdirHash;

    #[test]
    fn hash_converts_from_owned_and_borrowed_strings() {
        let owned = PimdirHash::from(String::from("abc"));
        let borrowed = PimdirHash::from("abc");
        assert_eq!(owned, borrowed);
        assert_eq!(owned.as_str(), "abc");
    }
}
