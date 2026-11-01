//! # Content hash
//!
//! The content hash naming a body in the object store (STORAGE §5).
//!
//! An object's name is its hash, so every process touching one store must
//! compute the same value: a disagreement writes blobs no other reader
//! finds, and it fails silently, as a dedup that never dedups. The format
//! names one algorithm, BLAKE3 at its default 32 bytes, which the store
//! records in `store_meta.hash_algo` (§4.3).
//!
//! The encoding is part of the contract: lowercase base32 (RFC 4648, no
//! padding), because the hash is also a path component and a
//! single-case, filesystem-safe alphabet is what keeps that path valid
//! everywhere. Hex would work on Linux and collide on a
//! case-insensitive filesystem.

use alloc::{boxed::Box, string::String};

use crate::object::PimdirHash;

/// The algorithm `store_meta.hash_algo` names (STORAGE §5): a store naming
/// another is refused.
pub const HASH_ALGO: &str = "blake3";

/// The content hash of a whole body.
pub fn hash(bytes: &[u8]) -> PimdirHash {
    let mut hasher = PimdirHasher::new();
    hasher.update(bytes);
    hasher.finish()
}

/// An incremental BLAKE3 hasher over a body's bytes, for a body streamed
/// into the blob store rather than held whole in memory (STORAGE §14's
/// byteless `StoreObject`).
///
/// Its state is boxed: it is nearly two kilobytes, which would otherwise
/// size every value holding a hasher.
pub struct PimdirHasher(Box<blake3::Hasher>);

impl PimdirHasher {
    /// A hasher over no bytes yet.
    pub fn new() -> Self {
        Self(Box::new(blake3::Hasher::new()))
    }

    /// Feeds the next bytes of the body.
    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    /// The finished hash, as the object store names it.
    pub fn finish(self) -> PimdirHash {
        PimdirHash(base32(self.0.finalize().as_bytes()))
    }
}

impl Default for PimdirHasher {
    fn default() -> Self {
        Self::new()
    }
}

/// Lowercase base32 (RFC 4648, no padding), the encoding spec §5 fixes
/// for an object name.
///
/// A digest length is rarely a multiple of five bits, so the last
/// character carries the leftover bits padded with zeroes, which is what
/// RFC 4648 prescribes once its padding characters are dropped.
fn base32(digest: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

    let mut name = String::with_capacity(digest.len().div_ceil(5) * 8);
    let mut buffer: u16 = 0;
    let mut bits = 0;

    for byte in digest {
        buffer = (buffer << 8) | u16::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            name.push(ALPHABET[usize::from((buffer >> bits) & 0x1f)] as char);
        }
    }
    if bits > 0 {
        name.push(ALPHABET[usize::from((buffer << (5 - bits)) & 0x1f)] as char);
    }

    name
}

#[cfg(test)]
mod tests {
    use crate::hash::*;

    #[test]
    fn base32_encodes_rfc_4648_vectors_lowercased() {
        // RFC 4648 §10, lowercased and unpadded
        assert_eq!(base32(b"f"), "my");
        assert_eq!(base32(b"fo"), "mzxq");
        assert_eq!(base32(b"foo"), "mzxw6");
        assert_eq!(base32(b"foob"), "mzxw6yq");
        assert_eq!(base32(b"fooba"), "mzxw6ytb");
        assert_eq!(base32(b"foobar"), "mzxw6ytboi");
    }

    #[test]
    fn a_streamed_body_hashes_like_a_whole_one() {
        let mut hasher = PimdirHasher::new();
        hasher.update(b"BEGIN:VCARD\r\n");
        hasher.update(b"UID:x\r\nEND:VCARD\r\n");
        assert_eq!(
            hasher.finish(),
            hash(b"BEGIN:VCARD\r\nUID:x\r\nEND:VCARD\r\n")
        );
    }

    #[test]
    fn a_name_is_52_characters_of_the_alphabet() {
        let name = hash(b"pimdir").0;
        assert_eq!(name.len(), 52);
        assert!(name.chars().all(|c| matches!(c, 'a'..='z' | '2'..='7')));
    }
}
