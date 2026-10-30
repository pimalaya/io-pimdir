---
cairn: change
id: a-streamed-body-is-verified-at-its-length
status: landed
created: 2026-10-30
---

# A streamed body is verified at its length

## Why

`PimdirBlobReader` verified the hash only on the read meeting the end of the file. A connector streaming an upload of a known size reads exactly `len()` bytes and never makes that read, so a corrupted body passed as sound (STORAGE §7).

## What

The reader records the length the file stated when opened (`len()` answers it) and the bytes read: the read reaching that length verifies the hash, failing with `InvalidData` on a mismatch; the end of the file before it verifies as before; a byte past it fails, the body being no longer than it stated.
