---
cairn: tasks
change: incremental-write-refcount
---

- [x] Add SQL: `ADJUST_REFCOUNT`, `UPDATE_ITEM`, `DELETE_ITEM` (single),
      `UPDATE_BINDING`, `DELETE_BINDING` (single). Remove `RECOMPUTE_REFCOUNTS`
      and `DELETE_ITEMS`.
- [x] `write`: snapshot old hub, clone for the new hub, absorb, diff-save, apply
      refcount deltas; keep GC.
- [x] `object_refs(hub)`: object-reference multiset (item object + conflict_object,
      binding base_object).
- [x] `save_hub` → `save_hub_diff(old, new)`: insert/delete/update only changed
      items and bindings.
- [x] Existing roundtrip tests pass (GC, two-source copy/delete, streaming).
- [x] Add a test: dedup refcount (two items sharing one blob; drop one keeps the
      blob, drop both GCs it) and a flag-only update leaves the blob.
- [x] fmt + clippy clean; measure the first-sync `write` curve is now ~linear.
- [x] Fold delta into `cairn/spec/store.md`; write log entry.
