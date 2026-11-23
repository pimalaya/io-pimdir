---
cairn: delta
change: store-audit-2026-10-30
---

## ADDED Requirements

### Requirement: The journal mode is chosen when the store is created
WAL at creation; no handle sets the journal mode of an existing store.

### Requirement: A purge cutoff is written as the stamps are
`purge_retained_before` refuses a cutoff not in `%Y-%m-%dT%H:%M:%fZ` as `cutoff-invalid`.

## MODIFIED Requirements

### Requirement: A producer stages its own body
The staging lock is held for the staging window alone, not the producer's lifetime.
