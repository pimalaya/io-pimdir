---
cairn: change
id: incremental-write-refcount
status: landed
created: 2026-08-01
---

# Incremental hub save and refcount maintenance (kill the per-write O(N²))

## Why

Syncing a large mailbox burns enormous CPU in the storage layer. Profiling a
first sync (neverest → Stalwart) showed the `write` seam going O(N²): 0.11s at
1k messages, 2.55s at 5k, 12.0s at 10k — a clean quadratic. On a real
tens-of-thousands-of-messages mailbox that is hundreds of CPU-seconds, dwarfing
the actual network transfer.

Two things in `write` scale badly:

1. **`RECOMPUTE_REFCOUNTS` runs on every write batch** and rescans *every object
   against every item and binding* from scratch:
   ```sql
   UPDATE objects SET refcount =
       (SELECT count(*) FROM items i WHERE i.object_hash = objects.hash OR i.conflict_object = objects.hash)
     + (SELECT count(*) FROM bindings b WHERE b.base_object = objects.hash)
   ```
   That is O(objects × items) per write, and the `OR` across two columns defeats
   the indexes. This is the quadratic — pure SQLite CPU, which is why it surfaces
   as user time.

2. **`save_hub` rewrites the whole collection on every write** (delete-all items
   then re-insert every item and binding), so even an incremental sync that
   changed one message rewrites all N rows, several times per mailbox per run.

mbsync (studied for comparison) never recomputes global state: it journals
per-record deltas and rewrites its flat state once per run. The lesson is *touch
only what changed*.

## What

Make both the refcount and the hub save **incremental**, keeping the exact same
observable semantics (refcounts, GC, crash safety, the one-transaction rule):

- **Refcount by delta.** `write` already loads the old hub before absorbing. Snap
  the object-reference multiset (item `object_hash` + `conflict_object`, binding
  `base_object`) before and after `absorb`, and apply only the per-hash
  differences with an indexed `UPDATE objects SET refcount = refcount + :delta
  WHERE hash = :hash`. Drop the global `RECOMPUTE_REFCOUNTS`. Cross-collection
  correct: a delta reflects only this collection's change, other collections'
  references stay counted.
- **Diffed hub save.** Replace `save_hub`'s delete-all/re-insert-all with a diff
  of old vs new hub: insert added items (and their bindings), delete removed items
  (bindings cascade), and for items present in both, update the row only if its
  columns changed and insert/update/delete only the bindings that changed. Both
  `ReplicaHubItem` and `ReplicaSourceBinding` derive `Eq`, so equality is exact.

Garbage collection (list zero-refcount objects, drop rows in-transaction, unlink
blobs after commit) is unchanged and now fed by accurate incremental refcounts.
Net effect: `write` becomes O(changed rows) in SQL instead of O(N²), and an
incremental sync stops paying O(mailbox).
