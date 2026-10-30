---
cairn: log
change: a-streamed-body-is-verified-at-its-length
date: 2026-10-30
---

# A streamed body is verified at its length

`PimdirBlobReader` verified a body only on the read meeting the end of the file, which a connector reading exactly `len()` bytes never makes, so a corrupted body streamed as sound.

## What landed

- src/client/blobs.rs: the reader records the length the file stated when opened (`len()` answers it) and the bytes read; the read reaching that length verifies the hash (`InvalidData` on a mismatch), the end of the file before it too, and a byte past it fails the read.
- Test: tests/store_rules.rs `a_streamed_body_read_to_its_length_is_verified`, failing before: `read_exact` of `len()` corrupted bytes answered them.
- Capabilities moved: store ("A body may be ingested and emitted by streaming", modified).
