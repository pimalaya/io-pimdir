---
cairn: delta
change: a-binding-conflict-persists-its-body
---

## ADDED Requirements

### Requirement: A conflict body is pinned
An object referenced by a binding's `conflict_object` SHALL be reachable, and SHALL NOT be collected while the binding stays conflicted. The pin SHALL be released when the conflict resolves, so a resolved conflict's body is collected like any other unreferenced object.

A conflict is resolved by a person, and the interval between the run that found it and the moment that person sits down is the whole point of persisting it. An unpinned body does not survive that interval, and its loss is silent: the resolver finds a revision naming a body that is gone, and can only fall back on asking the remote, which is the dependency the column exists to remove.

#### Scenario: A conflict body outlives a collection
- GIVEN a conflicted binding whose diverging body is stored
- WHEN the collector runs
- THEN the body is still readable

#### Scenario: Resolving releases the pin
- GIVEN the same binding, then resolved
- WHEN the collector runs
- THEN the body is collected like any other unreferenced object

### Requirement: Conflicted bindings are listable
The store SHALL answer for the conflicted bindings of an account without paging its collections, naming each one's collection, handle and link id, and the hashes of its base, local and diverging bodies.

The flag is written and read back with its row today and never filtered on, so the question "what is waiting for a decision" costs a full scan. A run reports that count on every invocation and a listing command asks it directly, so it is a question the store answers rather than one its callers assemble.

#### Scenario: Only the conflicted rows come back
- GIVEN a store holding conflicted and ordinary bindings across two collections
- WHEN its conflicted bindings are listed
- THEN every conflicted one is named with its three hashes, and no other row appears

## MODIFIED Requirements

### Requirement: A binding's unresolved conflict is persisted
Unchanged in what it requires of `conflicted` and `conflict_revision`. The `bindings` table SHALL additionally carry `conflict_object` (TEXT, nullable, referencing `objects(hash)`), round-tripped through `ReplicaSourceBinding` and, like `conflict_revision`, meaningful only while `conflicted` is set. It holds the diverging remote body at the revision beside it, so that resolution can read base, local and remote from the store alone.

This remains distinct from the item-level `conflicted` / `conflict_object`, which records a cross-source divergence; a store SHALL persist the two independently.

#### Scenario: The three bodies are readable without a remote
- GIVEN a conflicted binding written by a sync
- WHEN it is read back after a restart
- THEN its base, its local body and its diverging body are all readable from the store

## REMOVED Requirements

None.
