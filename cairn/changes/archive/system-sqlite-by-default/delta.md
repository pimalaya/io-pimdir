---
cairn: delta
change: system-sqlite-by-default
---

# Delta

## ADDED Requirements

### Requirement: SQLite is the system's unless `vendored` says otherwise
The `client` feature SHALL take `rusqlite` with no features of its own, so the default build links the SQLite the machine provides through pkg-config and a store this crate writes is the one the system's `sqlite3` reads. A `vendored` feature, off by default, SHALL forward to `rusqlite/bundled` for a static target or a platform carrying no library, mirroring what `vendored` does for OpenSSL across the org. No other feature SHALL imply either choice.

## MODIFIED Requirements

None.

## REMOVED Requirements

None.
