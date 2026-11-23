---
cairn: delta
change: a-streamed-body-is-verified-at-its-length
---

## MODIFIED Requirements

### Requirement: A body may be ingested and emitted by streaming
The read side verifies the body as the bytes read reach the length the file stated when opened, and at the end of the file before it; a byte past that length fails the read.
