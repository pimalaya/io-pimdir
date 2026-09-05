---
cairn: change
id: a-binding-conflict-persists-its-body
status: landed
created: 2026-08-28
---

# A persisted conflict remembers the revision and forgets the body

## Why

`persist-binding-conflict` gave the bindings table `conflicted` and `conflict_revision`, which is what the sync layer needs to stop re-deriving a push the remote already rejected. The engine now also carries the diverging remote body on the placement, so that resolution can happen outside the process that found it, in a program holding no credentials. The store is where that body has to survive a restart, and today there is nowhere to put it.

Two things follow from adding the column, and both are easy to miss.

The body has to be pinned. Objects are reachable through the bindings' base and the items' own hashes, and anything else is garbage. A conflict body written and not made reachable is collected between the run that found the conflict and the day the user sits down to resolve it, which is precisely the interval the whole design exists to span.

The conflicted bindings have to be findable. The flag is written and read back with its row today, never filtered on, so the only way to answer "what is waiting for me" is to page the whole store. A run that ends by telling the user how many conflicts are outstanding, and a command that lists them, both ask that question every time.

## What

- `conflict_object` on `bindings`, nullable, referencing `objects(hash)`, round-tripped through `ReplicaSourceBinding` and meaningful only while `conflicted` is set, exactly as `conflict_revision` already is.
- The column joins the reachability union the collector walks, and the pin is released when the conflict resolves.
- A query listing conflicted bindings across a store's collections, returning enough to name the item and read its three bodies.
- The schema change is folded into version 1 while the pimdir spec is draft, so an existing store is reconciled on open rather than refused.

The item-level `conflicted` and `conflict_object` pair, which records a cross-source divergence, is untouched and stays independent.
