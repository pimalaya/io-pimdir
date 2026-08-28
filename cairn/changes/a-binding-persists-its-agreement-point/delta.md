---
cairn: delta
change: a-binding-persists-its-agreement-point
---

## ADDED Requirements

### Requirement: A binding persists what it last agreed with the hub on
The `bindings` table SHALL carry `shared_object` (TEXT, nullable), round-tripped through `ReplicaSourceBinding`: the hub's shared body this source last reconciled against, which is the base of the cross-source merge. It is meaningful on every binding, conflicted or not, and `NULL` only until the source has folded once, where the sync base stands in for it.

This is the second base, for the second axis, and the store MUST keep both. `base_object` is what the source last agreed with its own **remote** and only a sync moves it, which is what keeps a pending push derivable; `shared_object` is what it last agreed with the **hub** and every live absorb moves it. Read one as the other and a source disagrees with itself: its own unpushed edit leaves the sync base behind the shared body exactly as another source folding in does, so the next edit is filed as a cross-source conflict and dropped. In memory the hub carries the field either way; the store is what makes it survive, and a conflicted absorb and the edit that settles it are typically different runs.

Unlike `conflict_object` beside it, `shared_object` SHALL NOT reference `objects(hash)`, SHALL NOT join the reachability union the collector walks, and SHALL NOT be refcounted. It is only ever compared for equality and never read as bytes, and a content hash compares the same after the body it named has been swept, so pinning would hold every body a source ever agreed with for the life of the binding and buy nothing.

#### Scenario: The agreement point survives a reopen
- GIVEN an ordinary unconflicted binding written by a sync
- WHEN it is read back after a restart
- THEN it carries the shared body the reconcile settled on

#### Scenario: A conflicted binding carries one too
- GIVEN a binding whose own remote diverged from it
- WHEN it is read back
- THEN its agreement point is there beside the conflict, the flag gating only the conflict pair

#### Scenario: A second offline edit across a restart is not a conflict
- GIVEN a store whose single source edited an item offline, with the push still pending
- WHEN the store is reopened and a second offline edit is absorbed
- THEN the hub adopts it and the item holds the second body

## MODIFIED Requirements

### Requirement: A store from an earlier draft of the current version is reconciled on open
While the pimdir spec is `draft`, a schema change MAY be folded into version 1
rather than added as a new version (spec §6). A store written by an earlier
draft is then stamped with the current `user_version` yet lacks the folded-in
columns, so the version check alone cannot detect it.

On open, the store SHALL reconcile its shape: every folded-in column found
missing SHALL be added (`ALTER TABLE … ADD COLUMN`, which requires the column to
be nullable or carry a constant default), guarded so the check is a no-op for an
up-to-date store, together with any index over a folded-in column. The set of
folded-in columns SHALL be kept complete as further columns are folded in;
`items.sort_key` and its `items_by_sort` index are part of it. Failing a later
query on a missing column is not acceptable. This requirement lapses when the
spec leaves `draft` and versions are frozen.

A column SHALL additionally be **backfilled** where `NULL` is not the value the
existing rows already imply. `bindings.shared_object` SHALL be set from its
item's own `object_hash`, so an upgraded store opens agreeing with the shared
body rather than reading as never having folded: added empty, it would have the
first absorb after the upgrade measure the cross-source axis from the sync base
again, which is one silent false conflict for every item whose push is pending.

## REMOVED Requirements

None.
