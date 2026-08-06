---
cairn: delta
change: read-only-open
---

# Spec delta

## ADDED Requirements

### Requirement: A reader can open the store read-only
`PimdirStore::open_read_only(dir, source)` SHALL open an existing store with `SQLITE_OPEN_READ_ONLY`: it never creates or migrates the database, and refuses a schema version other than the current one with the version error (a reader's SQL requires the current columns and never runs the migrations). The returned handle exposes the full read surface; any write through it fails at the SQLite layer.

## MODIFIED Requirements

## REMOVED Requirements
