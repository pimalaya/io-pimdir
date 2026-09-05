---
cairn: log
change: a-binding-persists-its-agreement-point
date: 2026-08-28
---

# A binding persists what it last agreed with the hub on

io-replica landed `a-source-cannot-diverge-from-itself`: the cross-source comparison had been made against the source's sync base, which only a sync moves, so a body the source itself folded in and had not pushed left the same gap another source folding in leaves. A second offline edit was filed as a conflict in a store with one source and no second source anywhere, and the edit resolving a conflicted binding went the same way, leaving the next run pushing the unmerged body over the remote the merge was made against.

`ReplicaSourceBinding::shared_object` is the fix, and until now this store dropped it on the floor. In memory the hub carries it and the fix works; the absorb that would file the conflict and the edit that settles it are different runs, days apart, and a column that is not persisted reads back `NULL`, `NULL` falls back to the sync base, and the fallback **is** the old answer. The upstream fix was inert here.

## What landed

`bindings.shared_object` (TEXT, nullable), round-tripped through `ReplicaSourceBinding` in `INSERT_BINDING`, `UPDATE_BINDING`, `binding_from_row` and all three bindings selects.

Two things about its shape are deliberate, and both read as oversights beside the column next to it.

It carries **no** `REFERENCES objects(hash)`, joins **no** index, and stays out of both the reachability union `RECOMPUTE_REFCOUNTS` and `REFCOUNT_DRIFT` walk and the `object_refs` multiset the write path diffs. `conflict_object` had to be counted because a resolver reads those bytes days later; this one is only ever compared for equality and never read at all, and a content hash compares the same after the body it named has been swept. Counting it would pin every body a source ever agreed with for the life of the binding, which is for ever, and buy nothing.

It is **not** gated on `conflicted`, where `conflict_revision` and `conflict_object` are. Those two describe an exception and are meaningless once it resolves. This describes the ordinary state of an ordinary binding, and gating it would erase the agreement point at exactly the moment the edit resolving a conflict needs it, dropping that edit as the divergence the change exists to stop.

## The backfill is the half that matters on the upgrade run

Folded into version 1 with the reconcile-on-open path, on the draft allowance (SPEC §6), so a store written before the column is healed rather than refused. Adding it empty is not healing it. An existing store's bindings sit behind the shared body wherever a push is pending, which is what a pending push *is*, so a `NULL` column would have the first absorb after the upgrade measure the cross-source axis from the sync base again: one silent false conflict for every item with an unpushed edit, on the run that upgrades. `BACKFILL_SHARED_OBJECT` sets each row from its item's own `object_hash`, in the same transaction as the `ALTER TABLE`, and §6 now says a folded-in column is backfilled wherever `NULL` contradicts what the rows already imply.

## Not in the conflict listing

`LIST_CONFLICTED_BINDINGS` serves a resolver, and it names the three bodies a merge is between. The agreement point is not a fourth body: it is engine bookkeeping about which of them this source last saw, and a person merging two vCards has no use for it. `item show` prints it instead, unconditionally, beside the base it is not, which is where an operator asking why a placement stopped moving is already looking.

## Also

The canonical format moved with it, the schema being normative and checked by `spec_drift`: pimdir's migrations/0001_init.sql, queries/bindings.sql (the four statements plus `backfill_shared_object`), and SPEC.md §4.3, §5, §6, §10 and §13.

## Capabilities moved

- **store**: a binding persists what it last agreed with the hub on, and an earlier-draft store gains the column backfilled rather than empty.
