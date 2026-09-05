---
cairn: tasks
change: a-binding-conflict-persists-its-body
---

- [x] Add `conflict_object` to `bindings`, nullable, referencing `objects(hash)`
- [x] Round-trip it through `ReplicaSourceBinding` beside `conflicted` and `conflict_revision`
- [x] Add it to the object reachability union the collector walks
- [x] Index it, as the item-level column already is
- [x] Fold the column into version 1 and reconcile an earlier-draft store on open
- [x] Add a query listing conflicted bindings across collections
- [x] Test: a conflict body survives a collection pass
- [x] Test: resolving releases the pin and the next pass collects the body
- [x] Test: the listing names every conflicted binding and nothing else
- [x] Test: a store written before the column opens and gains it
