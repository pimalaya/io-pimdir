---
cairn: log
date: 2026-08-24
change: own-the-content-hash
---

# The store owns the content hash

`store_meta.hash_algo` said `blake3` for every store this crate created, and no consumer computed one: cardamum, neverest and himalaya each carried a 128-bit FNV-1a rendered as hex, and the Android app computes `sha256-128` as lowercase base32. So the column that exists to make the digest discoverable was false, the digest was not the cryptographic hash the format calls for, the encoding was not the one an object path is specified to use, and a store written by one implementation and read by the other agreed on nothing.

`PimdirHashAlgo` now implements both admitted algorithms with the specified encoding, and a store, a producer and the algorithm itself hand the digest out, whole or incremental. The algorithm is declared when a store is created, recorded truthfully, adopted on reopen, and a handle declaring a different one is refused: a handle that hashes differently writes blobs nothing else finds, and it fails silently, as a dedup that never dedups.

Downstream, the three Rust consumers drop their local digest. Their existing stores are recreated, since every object name changes.

Capabilities moved: store.
