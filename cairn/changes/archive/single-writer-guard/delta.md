---
cairn: change
change: single-writer-guard
---

# Delta

## ADDED Requirements

### Requirement: Writes take the single writer lock up front
A `write` batch SHALL begin with `BEGIN IMMEDIATE`, taking the store's single
writer lock (SPEC §7) at the start of the transaction. Under WAL readers still
never block; two concurrent writers serialise on the connection's busy timeout,
and a writer that cannot acquire the lock within it SHALL fail with a clear
`PimdirError::Busy` (at begin or commit) rather than a raw SQL error or a failure
deep inside the batch. Coordinating who writes (a single owning process, or a
front daemon fronting the store for both a UI and a sync) remains a platform
decision, not enforced by the store.

## MODIFIED Requirements

## REMOVED Requirements
