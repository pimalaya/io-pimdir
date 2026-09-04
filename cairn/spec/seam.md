---
cairn: spec
capability: seam
status: current
---

# Seam

The storage seam is the local index plus blob store the engine reconciles against. The consumer implements it (a sqlite index plus a blob dir in the reference driver); the engine reads it through `load` and `lookup_objects` and mutates it through a `write` batch of [`PimdirWriteOp`]s. The engine performs no I/O of its own: storage effects travel as coroutine yields.

### Requirement: DropPlacement is the retention decision point
The engine SHALL signal every removal, a local delete confirmed by the remote or a member vanished upstream, as a `PimdirWriteOp::DropPlacement`, and SHALL make no other assumption about what the storage does with it. A storage MAY hard-delete the row, or soft-delete it (retain the row, marked removed-upstream) for a backup that must never lose a copy the source expunged. No merge-core option governs this; retention is the storage's choice.

### Requirement: Hiding rows from load is safe
The merge reconciles only the placements a `load` returns. A storage that hides soft-deleted rows from `load` SHALL therefore not cause the engine to re-derive against them: the hidden row is invisible on every later sync, delta or full, so it is neither re-added nor looped over, and the retained copy survives.

#### Scenario: A backup keeps a remote expunge
- GIVEN a storage that soft-deletes on `DropPlacement` and hides such rows from `load`
- AND a member the remote expunges after a first sync
- WHEN the collection is synced again, whether delta or full
- THEN the row is retained but absent from `load`, and no re-derivation occurs

### Requirement: Object write carries bytes or a reference
`PimdirWriteOp::StoreObject` SHALL carry either the object's bytes or a reference to an object the consumer has already persisted to the blob store. The engine SHALL index the object from its `(hash, size)` in both cases, and SHALL have the storage write bytes only when they are present. Refcounting, dedup and garbage collection are unaffected.

### Requirement: A Full fetch may persist its own body
A consumer servicing a `PimdirTier::Full` fetch MAY stream the body straight into the blob store and report the object by `(hash, size)` with no inline bytes, so no full body is held in memory. The engine SHALL treat such a report as an already-stored object and emit a byteless `StoreObject`. `lookup_objects`, which lets the engine skip a Full fetch whose object already exists, is unchanged, so a persist-during-fetch stays idempotent under retry.

#### Scenario: A large body transfers without full materialisation
- GIVEN a source whose Full fetch streams the body into the blob store and reports it by (hash, size)
- WHEN the item is hydrated and later appended to another source
- THEN the engine holds no full body, indexes the object from its (hash, size), and the append streams from the stored blob

### Requirement: A placement carries a presentation sort key
`PimdirPlacement` and `PimdirFetchedItem` SHALL carry a sort key: the item's position in its collection's natural order, derived by the consumer wherever the summary is derived. The engine SHALL treat it exactly as it treats the summary, as an opaque value it ferries and never parses.

Empty SHALL mean unknown, and SHALL be the default, so an item is orderable from the moment it exists and no consumer has to invent a value. It is a plain value rather than an option because that is what the reference storage records, and one representation of "not known yet" is less to get wrong than two.

### Requirement: An unread flag set is unknown rather than empty
`PimdirFlags` SHALL carry an `Unknown` state distinct from a known-empty set, since the reference storage records the two apart (pimdir STORAGE §13: a `NULL` flags column means never read, `'[]'` means known to carry none). Known-empty SHALL remain the default, so unknown is stated by a source that read no markers rather than fallen back to by an ordinary write.

Only a local placement is ever unknown in practice: a source that reports an item reports what it read. The engine SHALL resolve the state on the first side that carries a set.

### Requirement: An unknown side holds no opinion in the merge
The flag merge SHALL treat an unknown side as neither an addition nor a removal: the result is the other side's set, two unknown sides stay unknown, and an unknown base is the same fact as no base on the flag axis, so nothing is derived from it and both sides' markers are kept.

Reading unknown as empty would make it an opinion, since element-wise an empty set says every flag the other side holds was removed here.

#### Scenario: A probed placement learns its markers
- GIVEN a local placement whose flag set is unknown
- AND a source reporting that item with a marker set
- WHEN the collection is synced
- THEN the placement adopts the reported set and nothing is pushed

### Requirement: An unknown set never erases a known one
Absorbing an upsert whose flag set is unknown SHALL leave the shared set alone, on the same terms as an absent summary and an unknown sort key. A known set, empty or not, SHALL replace another known set: only unknown is inert.

### Requirement: A drop says whether the item is gone
`PimdirWriteOp::DropPlacement` SHALL carry a `PimdirDropReason`: `Deleted` when the item itself is gone (a local delete the remote confirmed, a member that vanished upstream), `Superseded` when only this row is gone, replaced by the server-assigned handle an accepted add reconciled a provisional placeholder to, and `Rekeyed` when a rebuild renumbered it onto a new handle space (pimdir SYNC §10). Both license the binding to move; only `Rekeyed` is the store's signal to bump the collection's generation.

This refines the retention decision point above rather than replacing it: a storage still chooses what a removal means for its rows, but a storage that shares one item across sources SHALL propagate a delete only for `Deleted`. Reading a superseded row as a delete turns housekeeping into data loss on every other source, and it is what made the drop-then-upsert order of a rebuilt spine load-bearing.

#### Scenario: A rebuilt spine is not a mass delete
- GIVEN an item bound to two sources
- WHEN one source's row is dropped as `Superseded`
- THEN the item is not marked deleted and the other source projects it unchanged

### Requirement: A load states what it needs
`PimdirYield::WantsLoad` SHALL carry a `PimdirLoadScope`: `All`, `Handles`, or `Links`. A mutation asks for the one placement it edits, or, for an `Add`, every row holding the link id it must not collide with; an upgrade asks for the handles it raises; only the merge and the rebuild ask for the whole collection, because only they reason about what is missing from it.

A load names the collection it reads, which is the coroutine's own except where a verb acts across two: a `Copy` or a `Move` reads its target for the identity it is carrying into it (mutate.md), that being the one question the collection it edits cannot answer.

The scope is a floor, not a ceiling: a storage SHALL return at least the placements it names and MAY return more, so a storage that ignores it stays correct, just as expensive as before. Under-delivering is not correct: a mutation that cannot see a colliding link id creates a duplicate.

#### Scenario: A one-row edit reads one row
- GIVEN a storage that honours the scope exactly
- WHEN any mutation other than `Add` runs
- THEN it is served only the placement it names, and still produces the same writes

### Requirement: A write batch is applied in order
The store's write SHALL apply the ops in the order they are listed, and atomically. Ordering is what a batch naming one handle twice rests on, which the engine does emit: a sync that resurrects a locally deleted item writes the placeholder, pushes it, and supersedes the same handle once the remote assigns one, and the drop is the answer.

A storage MAY NOT group the batch by op kind, or otherwise reorder it: what looks like an independent set of rows is not one.

The engine SHALL NOT rely on the order where it can avoid emitting the pair at all. A rebuild in particular drops only the old handles no upsert of the same batch writes: a new handle space commonly reuses an old handle, and an unmatched staged edit is resurrected under the handle it already had, so the collision there is between ops far apart in a long list, and the reasoning that they are safe lives in neither of them.

#### Scenario: A rebuilt spine reuses a handle
- GIVEN a placement the old handle space held under a handle the new one reuses
- WHEN the collection is rebuilt
- THEN the batch writes that handle once, and does not also drop it
