---
cairn: change
id: a-binding-persists-its-agreement-point
status: landed
created: 2026-08-28
---

# The fix upstream is inert until the store keeps the column

## Why

io-replica landed `a-source-cannot-diverge-from-itself`. The hub used to decide a cross-source divergence by comparing its shared body against the source's **sync** base, and that base only advances when the source reconciles against its own remote: a body this source folded into the hub and has not pushed yet leaves the same gap another source folding in leaves. A second offline edit was therefore filed as a conflict in a store with one source and no second source anywhere, and the edit resolving a conflicted binding went the same way, which left the next run pushing the unmerged body over the remote the merge was made against.

The fix gives each axis its own base. `ReplicaSourceBinding` now carries `shared_object`, the shared body this source last reconciled against, and the cross-source comparison is made against that.

Within one process the in-memory hub carries it and the fix works. Across runs it does not, and across runs is where the bug lives: a conflicted absorb and the edit that settles it are typically different runs, days apart. A column the store does not keep reads back `NULL`, `NULL` falls back to the sync base, and that fallback **is** the old answer. So the upstream fix does nothing here until the column is persisted, and neverest cannot drop the workaround it carries, which reaches into a binding's sync base to correct an answer on the hub axis.

## What

`bindings.shared_object`, TEXT, nullable, round-tripped through `ReplicaSourceBinding` in the insert, the update, the row reader and every bindings select.

It is deliberately **not** shaped like `conflict_object` beside it, and the asymmetry is the part a later reader will take for an oversight:

- No `REFERENCES objects(hash)`, no place in the reachability union the collector walks, no refcount, no index. The value is only ever compared for equality and never read as bytes, and a content hash compares the same after the body it named has been swept. Pinning it would keep every body a source ever agreed with alive for as long as the binding lives, which is for ever.
- Not gated on `conflicted`. `conflict_revision` and `conflict_object` describe an exception and are meaningless once it resolves; this describes the ordinary state of an ordinary binding, and gating it would erase the agreement point at exactly the moment a resolving edit needs it.

Folded into schema version 1 with the reconcile-on-open path, on the draft allowance (spec §6), and **backfilled** from the item's own `object_hash`. That backfill is not cosmetic: an existing store has bindings whose sync base sits behind the shared body, which is what a pending push looks like, so a column left `NULL` would make the first absorb after the upgrade read exactly the divergence the change exists to stop. One silent false conflict per item with an unpushed edit, on the upgrade run.

Not in the conflict listing. `list_conflicted_bindings` answers a resolver, which merges three bodies it can name; the agreement point is engine bookkeeping about which of them the source last saw, and a resolver has no use for it. `item show` prints it, where an operator asking why a placement stopped moving is asking about exactly this column.
