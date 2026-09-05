---
cairn: delta
change: sql-index
---

# Delta

## ADDED Requirements

### Requirement: The canonical SQL is reachable by name
`sql` SHALL expose `ALL`, a `&[(&str, &str)]` pairing every statement constant's
name with its text, `MIGRATION_0001` included and `VERSION` excluded. A consumer
without the `client` feature, holding its own SQLite driver, SHALL be able to
recover any statement from it by name without a per-statement accessor.

The index SHALL be covered by a test derived from the module's own source, so a
statement added without being indexed fails the suite rather than shipping a
silent gap.

## MODIFIED Requirements

None.

## REMOVED Requirements

None.
