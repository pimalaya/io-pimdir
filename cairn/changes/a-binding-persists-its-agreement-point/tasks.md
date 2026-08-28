---
cairn: tasks
change: a-binding-persists-its-agreement-point
---

- [x] Add `shared_object` to `bindings`, TEXT, nullable, referencing nothing
- [x] Round-trip it through `ReplicaSourceBinding`, ungated by `conflicted`
- [x] Keep it out of the reachability union, the refcount and the indexes, and say why where it is written
- [x] Fold it into version 1 and reconcile an earlier-draft store on open
- [x] Backfill it from the item's own body, so an upgraded store opens in agreement
- [x] Print it under `item show`, beside the base it is not
- [x] Test: the agreement point survives a reopen, on an ordinary unconflicted binding
- [x] Test: a conflicted binding carries one too, the flag not gating it
- [x] Test: a store written before the column gains it, backfilled rather than `NULL`
- [x] Test: a second offline edit across a restart is adopted, not filed as a conflict
- [x] Carry the schema into the canonical format repo and log it there
