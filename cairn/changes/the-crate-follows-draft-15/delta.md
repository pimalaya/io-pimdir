---
cairn: delta
change: the-crate-follows-draft-15
---

# Delta

## ADDED Requirements

### Requirement: The CLI is outside the crate's 1.0 contract
The binary's commands, flags and output SHALL be outside the semver contract io-pimdir 1.0 makes for its library (pimdir the-freeze-plan, D4), and `pimdir --help` and README.md SHALL say so: they may change in any release, so a program depends on the library, never on the binary.

## MODIFIED Requirements

### Requirement: The crate states its conformance
README.md SHALL state the crate's conformance as `storage-draft-15 owner, sync-draft-15 engine, search-draft-15 indexer (canonical) and query client`.

### Requirement: The index is derived and rebuilt, never migrated
An index at `search-draft-14` is rebuilt at `search-draft-15`; one at `search-draft-16` is refused. The search layer follows the crate's versioning, SEARCH.md being released together with STORAGE.md and SYNC.md.

## REMOVED Requirements

None.
