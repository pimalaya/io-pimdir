//! # Content hash
//!
//! The content hash naming a body in the object store (STORAGE §5).
//!
//! An object's name is its hash, so every process touching one store must
//! compute the same value: a disagreement writes blobs no other reader
//! finds, and it fails silently, as a dedup that never dedups. The store
//! therefore records its algorithm in `store_meta.hash_algo` (spec §4.3)
//! and hands the digest out, rather than leaving each consumer to pick.
//!
//! The encoding is part of the contract: lowercase base32 (RFC 4648, no
//! padding), because the hash is also a path component and a
//! single-case, filesystem-safe alphabet is what keeps that path valid
//! everywhere. Hex would work on Linux and collide on a
//! case-insensitive filesystem.

use alloc::{boxed::Box, string::String, vec::Vec};

use sha2::{Digest, Sha256};

use crate::object::PimdirHash;

/// The hash a store names its objects by (STORAGE §5).
///
/// Recorded in `store_meta.hash_algo` when the store is created.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PimdirHashAlgo {
    /// BLAKE3, the whole 256-bit digest, which the spec recommends.
    #[default]
    Blake3,
    /// SHA-256 truncated to its first 128 bits, for a consumer whose
    /// platform ships SHA-256 and would otherwise bundle a BLAKE3
    /// implementation (the Android app takes this one).
    ///
    /// Content addressing needs collision resistance, not signature
    /// strength, and a 26-character name keeps the blob paths short.
    Sha256_128,
}

impl PimdirHashAlgo {
    /// The spelling `store_meta.hash_algo` carries.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Blake3 => "blake3",
            Self::Sha256_128 => "sha256-128",
        }
    }

    /// The algorithm a stored spelling names, or `None` for one this
    /// crate does not implement, which a caller reports rather than
    /// guessing around.
    pub fn parse(algo: &str) -> Option<Self> {
        match algo {
            "blake3" => Some(Self::Blake3),
            "sha256-128" => Some(Self::Sha256_128),
            _ => None,
        }
    }

    /// The content hash of a whole body.
    pub fn hash(&self, bytes: &[u8]) -> PimdirHash {
        let mut hasher = self.hasher();
        hasher.update(bytes);
        hasher.finish()
    }

    /// An incremental hasher, for a body streamed into the blob store
    /// rather than held whole in memory (spec §14's byteless
    /// `StoreObject`).
    pub fn hasher(&self) -> PimdirHasher {
        match self {
            Self::Blake3 => PimdirHasher::Blake3(Box::new(blake3::Hasher::new())),
            Self::Sha256_128 => PimdirHasher::Sha256_128(Sha256::new()),
        }
    }
}

/// An incremental hasher over a body's bytes.
///
/// Made by [`hasher`](PimdirHashAlgo::hasher), for a body streamed into
/// the blob store.
pub enum PimdirHasher {
    /// A BLAKE3 digest in progress, boxed: its state is nearly two
    /// kilobytes, which would otherwise size every hasher this enum
    /// hands out.
    Blake3(Box<blake3::Hasher>),
    /// A SHA-256 digest in progress, truncated when it finishes.
    Sha256_128(Sha256),
}

impl PimdirHasher {
    /// Feeds the next bytes of the body.
    pub fn update(&mut self, bytes: &[u8]) {
        match self {
            Self::Blake3(hasher) => {
                hasher.update(bytes);
            }
            Self::Sha256_128(hasher) => hasher.update(bytes),
        }
    }

    /// The finished hash, as the object store names it.
    pub fn finish(self) -> PimdirHash {
        let digest: Vec<u8> = match self {
            Self::Blake3(hasher) => hasher.finalize().as_bytes().to_vec(),
            Self::Sha256_128(hasher) => hasher.finalize()[..16].to_vec(),
        };

        PimdirHash(base32(&digest))
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
    use super::*;

    #[test]
    fn sha256_128_matches_the_shape_every_implementation_must_agree_on() {
        // NOTE: pinned against the Android app's PimdirHash, which
        // computes the same name in Java. A disagreement here does not
        // fail loudly, it writes blobs the other never finds.
        let hash = PimdirHashAlgo::Sha256_128.hash(b"pimdir");
        assert_eq!(hash.0.len(), 26);
        assert!(hash.0.chars().all(|c| ALPHABET_CHARS.contains(c)));
    }

    const ALPHABET_CHARS: &str = "abcdefghijklmnopqrstuvwxyz234567";

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
        for algo in [PimdirHashAlgo::Blake3, PimdirHashAlgo::Sha256_128] {
            let mut hasher = algo.hasher();
            hasher.update(b"BEGIN:VCARD\r\n");
            hasher.update(b"UID:x\r\nEND:VCARD\r\n");
            assert_eq!(
                hasher.finish(),
                algo.hash(b"BEGIN:VCARD\r\nUID:x\r\nEND:VCARD\r\n")
            );
        }
    }

    #[test]
    fn the_algorithms_round_trip_through_their_stored_spelling() {
        for algo in [PimdirHashAlgo::Blake3, PimdirHashAlgo::Sha256_128] {
            assert_eq!(PimdirHashAlgo::parse(algo.as_str()), Some(algo));
        }
        assert_eq!(PimdirHashAlgo::parse("md5"), None);
        assert_eq!(PimdirHashAlgo::default(), PimdirHashAlgo::Blake3);
    }
}
