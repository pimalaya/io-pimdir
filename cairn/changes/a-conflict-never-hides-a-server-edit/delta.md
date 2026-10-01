---
cairn: delta
change: a-conflict-never-hides-a-server-edit
---

## ADDED Requirements

## MODIFIED Requirements

### Requirement: An item-level conflict is held, a binding conflict is reconciled (sync)
`reconcile_content` SHALL take its conflict branch for a placement carrying a `conflict_revision`, the divergence between this source and its own remote. A `Conflict` placement carrying none is the item's cross-source conflict projected onto this source ([hub](hub.md)): it SHALL derive no push, a pending create under it included, until an `Edit` or a `Remove` settles the item (pimdir SYNC §3, §7). A remote revision its base does not hold SHALL still be recorded, whatever the conflict policy: the binding is marked conflicted with that revision and its diverging body wanted, reported as `Conflicted`, since an incremental enumeration never lists the member again (pimdir SYNC §5).

#### Scenario: Two sources diverged under `manual`
- GIVEN an item flagged conflicted by the hub, every binding projecting `Conflict` with no revision
- WHEN either source is synced and its remote lists nothing new
- THEN nothing is pushed and nothing is re-marked, until an edit through one source settles it

#### Scenario: A server edit lands while the item is conflicted
- GIVEN an item flagged conflicted by the hub, and one source's remote listing the member at a revision past that binding's base
- WHEN that source is synced
- THEN the binding is marked conflicted with the listed revision, its body wanted, and the run reports one conflict

### Requirement: The offline mutation vocabulary (mutate)
The `Remove` bullet becomes: tombstone a placement, kept until synced. Absorbed as a staged delete (the item is marked deleted, its binding kept), so the next sync pushes the remove. A `Remove` on a conflicted placement settles the conflict: the base adopts `conflict_revision` and `conflict_object` together, as an `Edit` does, an unfetched diverging body leaving the base's object unknown, so the delete pushes against what the remote holds and a revived item never reads its remote as holding the body it held before (pimdir SYNC §7, vectors/sync/32); the item's own conflict clears when the tombstone is absorbed ([hub](hub.md)).

## REMOVED Requirements
