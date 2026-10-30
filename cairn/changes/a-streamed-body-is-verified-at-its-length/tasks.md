---
cairn: tasks
change: a-streamed-body-is-verified-at-its-length
---

- [x] Test: reading exactly `len()` bytes of a corrupted blob fails `InvalidData`, a body grown after the open fails too (tests/store_rules.rs `a_streamed_body_read_to_its_length_is_verified`; failed before: the corrupted bytes read `Ok`)
- [x] src/client/blobs.rs: `PimdirBlobReader` verifies at the stated length and at the end of the file
- [x] CHANGELOG; spec store.md folded; log
