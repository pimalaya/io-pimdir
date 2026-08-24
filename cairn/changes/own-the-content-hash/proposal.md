---
cairn: change
id: own-the-content-hash
status: landed
created: 2026-08-24
---

# The store owns the content hash it names objects by

## Why

An object's name is its hash, so every process touching one store must compute the same value. Nothing enforced that. This crate stamped `store_meta.hash_algo` with `'blake3'` unconditionally and hashed nothing itself, while its Rust consumers (cardamum, and by their own comments neverest and himalaya) each carried a copy of a 128-bit FNV-1a rendered as hex, and the Android app computes `sha256-128` as lowercase base32.

Three things were wrong at once. The recorded algorithm was a statement no consumer honoured, so the one column that exists to make the digest discoverable (spec §4.3) lied. FNV-1a is not the cryptographic hash the format calls for (spec §2), and hex is not the lowercase base32 an object path is specified to use (spec §5). And a store written by a Rust consumer and read by the Android app agreed on nothing: same body, two names, no dedup, no blob found.

The failure mode is the worst kind: silent. Nothing errors, the cache simply never hits and bodies accumulate under names the other side never looks up.

## What

The store owns the digest. `PimdirHashAlgo` implements both algorithms the format admits, encodes them as spec §5 requires, and is reachable from a store, a producer and on its own, so a consumer hashes through the store it is writing to rather than choosing for itself.

The algorithm is declared once, when the store is created, and recorded truthfully. Reopening adopts what the store records; declaring a different one is refused, because a handle hashing differently from the store's own names is the silent failure above.
