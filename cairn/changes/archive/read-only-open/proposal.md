---
cairn: change
id: read-only-open
status: landed
created: 2026-08-07
---

# Read-only store open

## Why

The connector split (pimgate serving stores neverest owns) needs a store handle that is provably unable to write: `PimdirStore::open` migrates on open and holds a writable connection, and `PimdirProducer` is write-capable by design. A frontend that only reads must open pimdir.db with `SQLITE_OPEN_READ_ONLY` so a bug cannot mutate the owner's store.

## What

Add `PimdirStore::open_read_only(dir, source)`: opens the existing database read-only (no create, no migration), refuses a schema version other than the current one (the read SQL needs the current columns), and returns the same `PimdirStore` read surface. Write paths on such a handle fail at the SQLite layer.
