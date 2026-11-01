//! # Object
//!
//! The content-addressed item body, stored once and shared by every
//! placement pointing at it.
//!
//! One of the two identity axes, next to [`crate::placement`]. An edit
//! of mutable content is a new object, the old one dereferenced. Many
//! placements sharing one object is the dedup and unified-view mechanism.

crate::pimdir_id! {
    /// The content hash naming an object in the object store (STORAGE §5).
    ///
    /// Its BLAKE3 digest in lowercase base32, 52 characters. The value is
    /// held as read: a name from a row, a payload, a directory entry or a
    /// remote is checked by [`is_name`](Self::is_name) before any path is
    /// built from it, which the blob store does on every path it builds.
    PimdirHash, Ord, PartialOrd, Hash,
}

impl PimdirHash {
    /// The length of a well-formed name (STORAGE §5).
    pub const LEN: usize = 52;

    /// Whether the hash is a well-formed name: [`LEN`](Self::LEN)
    /// characters of `a` to `z` and `2` to `7`, so no path built from it
    /// leaves its shard (STORAGE §5).
    pub fn is_name(&self) -> bool {
        is_name(&self.0)
    }
}

/// Whether `name` is a well-formed object name (STORAGE §5): what a
/// directory entry is checked by before it is read as a body.
pub(crate) fn is_name(name: &str) -> bool {
    name.len() == PimdirHash::LEN && name.bytes().all(is_name_byte)
}

/// Whether `byte` belongs to the name alphabet, lowercase RFC 4648 base32.
pub(crate) fn is_name_byte(byte: u8) -> bool {
    matches!(byte, b'a'..=b'z' | b'2'..=b'7')
}

/// A stored object: its content hash and byte size.
///
/// The bytes live out of band under objects/, in the shard directory its
/// name's first two characters name (STORAGE §5), refcounted so copy, move
/// and undelete are reference edits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirObject {
    /// The content hash naming the bytes.
    pub hash: PimdirHash,
    /// The byte size of the content.
    pub size: usize,
}

#[cfg(test)]
mod tests {
    use alloc::{format, string::String};

    use crate::object::PimdirHash;

    #[test]
    fn hash_converts_from_owned_and_borrowed_strings() {
        let owned = PimdirHash::from(String::from("abc"));
        let borrowed = PimdirHash::from("abc");
        assert_eq!(owned, borrowed);
        assert_eq!(owned.as_str(), "abc");
    }

    #[test]
    fn a_name_is_52_characters_of_lowercase_base32() {
        let name = "v4jutopvhnmlxtgwm3ymv5f4nxfy2u2ygr6xyx2tj2t3yrfgm2la";
        assert!(PimdirHash::from(name).is_name());

        for refused in [
            "../x",
            "",
            &name[..51],
            &format!("{name}a"),
            &name.to_uppercase(),
            &format!("{}1", &name[..51]),
            &format!("{}=", &name[..51]),
            &format!("{}/", &name[..51]),
            &format!("{}\\", &name[..51]),
            &format!("{}é", &name[..50]),
        ] {
            assert!(!PimdirHash::from(refused).is_name(), "{refused:?}");
        }
    }
}
