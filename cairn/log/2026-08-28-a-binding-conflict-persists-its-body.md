---
cairn: log
change: a-binding-conflict-persists-its-body
landed: 2026-08-28
---

# A binding conflict persists its body, and the collector is told about it

`persist-binding-conflict` gave the bindings table `conflicted` and `conflict_revision`, which is the sync layer's memory of a push its own remote already rejected. io-replica now carries the diverging body beside that revision on the placement, so resolution can happen in a program holding no credentials, and the store is where that body has to survive a restart. It had nowhere to go.

## What landed

`bindings.conflict_object` (TEXT, nullable, referencing `objects(hash)`), round-tripped through `ReplicaSourceBinding` and gated on the flag exactly as the revision beside it is: written through one helper, read back through one, so the two can never disagree about whether a resolved binding still holds a body. A body outliving its revision describes a version the remote no longer has, and a resolver merging against it would show the user a phantom.

The item-level `items.conflicted` / `items.conflict_object`, which records a cross-source divergence, is untouched. One says a source and its own server disagree, the other that two sources do, and a two-source store needs both.

## The half that would have been missed

An object is reachable through the columns that point at it and nothing else, so a body written and not counted is at refcount zero from the moment it lands. The first collection after the run that found the conflict would take it, and that is precisely the interval the whole design exists to span: a conflict is a person's decision, taken days later. The loss would also be silent, the resolver finding a revision naming bytes nobody holds and falling back on asking the remote, which is the dependency the column was added to remove.

So the column joins the reachability union `RECOMPUTE_REFCOUNTS` and `REFCOUNT_DRIFT` walk, and the in-memory `object_refs` multiset the write path diffs. The pin needs no release path of its own: resolving is an ordinary edit, the projection drops the body with the flag, and the write's own refcount diff takes the reference away. Five columns pin an object now rather than four, and the prose that counted them says so.

Two indexes: `bindings_by_conflict_object`, so the recomputation reaches the new pointer by index rather than by scanning bindings once per object, mirroring `items_by_conflict_object`; and `bindings_conflicted`, partial on the flag, for the listing below.

## What is waiting for a decision

The flag was written and read back with its row and never filtered on, so answering "how many conflicts are outstanding" meant paging every collection. A sync reports that number at the end of every run and a listing command asks it directly, so it is now a question the store answers: `LIST_CONFLICTED_BINDINGS`, scoped to one account, returning each binding's collection, link id, source and handle, its `conflict_revision`, and the three hashes the divergence is between: the base, the item's own body, and the remote body. `PimdirReader::list_conflicts` returns them as `PimdirConflict`. Reading all three from one row is what lets a resolver hold no credentials.

The row names its source as well as its item, because a store with two sources has two bindings per item and one of them can be conflicted while the other is in sync.

## Schema, not a version

Folded into version 1 rather than added as version 2, which the draft allowance (SPEC §6) permits and which means a store written before the column is stamped current and lacks it. `reconcile_draft_shape` gains the entry, so such a store is healed on open rather than failing on the next read. The reconciliation adds columns before it ensures indexes, which is the order the partial index over `conflicted` needs.

One consequence surfaced in the tests: SQLite refuses to drop a column a partial index names, so rewinding a store to the earlier draft's shape has to drop the two indexes first. That is honest rather than awkward, a draft without the columns not having carried the indexes either.

## Also

`item show` prints the conflict object beside the revision, on the same rule as the rest of that block: only when the binding is conflicted, so an exception still reads as one.

The canonical format moved with it, the schema being normative and checked: pimdir's migrations/0001_init.sql, queries/bindings.sql, queries/objects.sql and SPEC.md §5, §7, §13 and §14.1 carry the column, the two indexes, the fifth reference and the new statement.

## Capabilities moved

- **store**: a binding's persisted conflict carries the diverging body; that body is pinned against the collector until the conflict resolves; the conflicted bindings of an account are listable without paging its collections.
