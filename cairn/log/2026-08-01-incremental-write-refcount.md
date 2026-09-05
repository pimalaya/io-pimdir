---
cairn: log
change: incremental-write-refcount
date: 2026-08-01
---

# Incremental hub save and refcount maintenance (kill the per-write O(N²))

`write` was O(N²) per batch and made large-mailbox sync burn hundreds of
CPU-seconds. Two causes, both fixed:

1. **Global refcount recompute.** `RECOMPUTE_REFCOUNTS` rescanned every object
   against every item and binding on every write (`O(objects × items)`, the `OR`
   defeating the indexes). Replaced with an in-memory per-hash delta: `write`
   already loads the old hub, so we snapshot the object-reference multiset (item
   `object` + `conflict_object`, binding `base.object`) before and after `absorb`
   and issue `UPDATE objects SET refcount = refcount + :delta WHERE hash = :hash`
   only for hashes whose count moved. Cross-collection correct (a batch adjusts by
   this collection's change alone). `RECOMPUTE_REFCOUNTS` removed.

2. **Whole-collection rewrite.** `save_hub` did delete-all-items + re-insert-all
   on every batch, so an incremental sync rewrote all N rows several times.
   Replaced with `save_hub_diff(old, new)`: insert added items (+bindings), delete
   removed items (bindings cascade), and for items in both, update the row only if
   its columns changed and insert/update/delete only the bindings that changed.
   `ReplicaHubItem`/`ReplicaSourceBinding` derive `Eq`, so the comparison is exact.
   New SQL: `UPDATE_ITEM`, `DELETE_ITEM`, `UPDATE_BINDING`, `DELETE_BINDING`,
   `ADJUST_REFCOUNT`; `DELETE_ITEMS` removed.

GC (list zero-refcount objects, drop rows in-transaction, unlink blobs after
commit) is unchanged, now fed by accurate incremental refcounts.

Measured (neverest → Stalwart, release, first sync): the `write` seam's
per-batch time went from O(N²) — 0.11s / 2.55s / 12.0s at 1k / 5k / 10k messages
— to ~O(N): **12.0s → 1.2s at 10k**, and total sync user CPU 10.2s → 2.0s. An
incremental sync of one new message into a 10k mailbox now writes in ~0.1s
instead of rewriting all 10k rows; a no-change re-sync writes nothing.

Tests: the existing roundtrip suite (GC, two-source copy/delete, streaming) still
passes; added `a_shared_blob_survives_until_its_last_referrer_is_dropped` (dedup
refcount: two items sharing a blob, dropped one at a time) and
`a_flag_only_update_keeps_the_body` (in-place item update leaves the object).
fmt/clippy clean.

Spec updated: `store` (MODIFIED: A write batch is one transaction — now diffed
save + incremental refcount, O(changed rows)).
