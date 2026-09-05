---
cairn: delta
change: own-the-content-hash
---

## ADDED Requirements

### Requirement: The store owns the content hash
The crate SHALL implement the hashes the format admits (spec §4.3: `blake3`, recommended, and `sha256-128`) and encode them as spec §5 requires, in lowercase base32 (RFC 4648, no padding), since the hash is also a path component and a single-case, filesystem-safe alphabet is what keeps the blob path valid everywhere.

A store, a producer and the algorithm itself SHALL expose the digest, whole and incremental, so a consumer hashes through the store it writes to instead of choosing an algorithm of its own. An object's name is its hash: two processes disagreeing about it write bodies neither finds and dedup against nothing, and nothing errors while they do it.

### Requirement: A store declares its algorithm once and is refused on a mismatch
The algorithm SHALL be recorded in `store_meta.hash_algo` when the store is created, and every blob being a file named by it, it cannot change afterwards. An open SHALL adopt what an existing store records; an open declaring a different algorithm, or meeting one this crate does not compute, SHALL be refused with `PimdirError::HashAlgo` rather than return a handle that names bodies the store does not use.
