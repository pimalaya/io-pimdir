---
cairn: tasks
change: own-the-content-hash
---

# Tasks

- [x] `PimdirHashAlgo` with `blake3` and `sha256-128`, and lowercase base32 (RFC 4648, no padding) encoding.
- [x] `PimdirHasher`, the incremental form, for a body streamed into the blob store.
- [x] `open_with_hash` declares the algorithm a created store records; `open` adopts or defaults.
- [x] An existing store whose algorithm differs from the declared one is refused with `PimdirError::HashAlgo`.
- [x] `hash_algo`, `hash` and `hasher` on the store and on the producer.
- [x] Tests: the RFC 4648 vectors; a streamed body hashes like a whole one; the declared algorithm is recorded, adopted on reopen and refused when it disagrees.
- [x] CHANGELOG.
- [x] Fold `delta.md`; log; land.
- [ ] **Downstream, cardamum, neverest and himalaya**: drop the local FNV-1a digest and hash through the store. Their existing stores are recreated, since every object name changes.
