---
cairn: change
change: persist-binding-conflict
---

# Delta

## ADDED Requirements

### Requirement: A binding's unresolved conflict is persisted
The `bindings` table SHALL carry `conflicted` (INTEGER, `0`/`1`) and
`conflict_revision` (TEXT, nullable), and the store SHALL round-trip both
through `ReplicaSourceBinding`. `conflict_revision` SHALL be written and read as
meaningful only while `conflicted` is set, so a resolved binding cannot hand a
stale revision to the next sync.

This is distinct from the item-level `conflicted` / `conflict_object`, which
records a cross-source divergence; a store SHALL persist the two independently.
Without the binding pair the sync layer loses its memory of an unresolved
conflict across a restart, re-derives the push its remote already rejected on
every run, and never converges.

### Requirement: A store from an earlier draft of the current version is reconciled on open
While the pimdir spec is `draft`, a schema change MAY be folded into version 1
rather than added as a new version (spec §6). A store written by an earlier
draft is then stamped with the current `user_version` yet lacks the folded-in
columns, so the version check alone cannot detect it.

On open, the store SHALL reconcile its shape: every folded-in column found
missing SHALL be added (`ALTER TABLE … ADD COLUMN`, which requires the column to
be nullable or carry a constant default), guarded so the check is a no-op for an
up-to-date store. Failing a later query on a missing column is not acceptable.
This requirement lapses when the spec leaves `draft` and versions are frozen.

## MODIFIED Requirements

None.

## REMOVED Requirements

None.
