---
cairn: delta
change: duplicate-link-id-freeze
---

# Delta

## ADDED Requirements

### Requirement: A binding never changes handle silently
A write that resolves an existing `(collection, link_id, source)` to a different
handle SHALL NOT repoint the binding. The store holds one item per identity per
collection, so such a write is a source holding the identity twice, and
performing it as an ordinary update destroys the only evidence of the second
copy: everything downstream then treats the surviving handle as the identity,
and a delete of it propagates as if the source no longer held the item at all.

The bound handle SHALL be kept and the incoming one recorded as ambiguous
instead. The rule holds for every path that reaches the table, including a
queued action applied by the owner, since not every write passes through the
sync engine's own identity resolution.

### Requirement: An ambiguous binding is persisted
`bindings` SHALL carry `ambiguous_handles`, the handles a source holds for the
identity beside the bound one, written by `write`, returned by `load` and
round-tripped through `ReplicaSourceBinding`. `NULL` and an empty array both
mean none.

Persistence is what makes the engine's freeze survive: the second copy appears
in exactly one enumeration, the one that discovers it, and an incremental
enumeration never mentions it again, so a state held only in memory would expire
on the next run and leave the identity deletable.

The store SHALL take no position on what the multiplicity means, on the same
terms as `link_placements` and `object_placements`: it records the handles and
resolves nothing.

#### Scenario: A colliding write is recorded, not applied
- GIVEN a binding of an identity to one handle
- WHEN a write resolves the same `(collection, link_id, source)` to another handle
- THEN the bound handle is unchanged and the incoming handle is recorded as ambiguous

#### Scenario: The ambiguity survives a reopen
- GIVEN a binding carrying ambiguous handles
- WHEN the store is closed and reopened
- THEN `load` returns the same handles on the binding

## MODIFIED Requirements

## REMOVED Requirements
