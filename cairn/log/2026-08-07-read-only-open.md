---
cairn: log
change: read-only-open
landed: 2026-08-07
---

# Read-only store open

Added `PimdirStore::open_read_only(dir, source)` for the connector split
(pimgate serving stores neverest owns): opens the existing pimdir.db with
`SQLITE_OPEN_READ_ONLY`, never creates or migrates, refuses any schema version
other than the current one with `PimdirError::Version`, and returns the normal
`PimdirStore` read surface. Writes through such a handle fail at the SQLite
layer, so a frontend bug cannot mutate the owner's store. Covered by
tests/read_only.rs (owner writes, reader reads, reader's writes refused,
missing database refused).

Capabilities moved: store (one ADDED requirement).
