---
cairn: tasks
change: action-queue
---

# Tasks

- [x] Add the queue and generation DDL to the sql module schema, byte-consistent with the canonical file.
- [x] Producer enqueue API: single-transaction ensure_collection + optional object upsert + queue insert; document the blob-first rule.
- [x] Owner drain API: list queued collections, load pending actions in append order, apply + delete in one transaction, attempts increment, parking with `error`, parked listing.
- [x] Action payload codec (no_std): the six v1 kinds, seq addressing, versioned JSON round-trip.
- [x] Refcount: pin `queue.object_hash` in the incremental scheme (+1 enqueue, released at row delete).
- [x] Generation: bump in rebuild transactions, expose on the read surface.
- [x] Read surface: pending-actions overlay per collection.
- [x] Tests: enqueue/drain round-trip per action kind, idempotent reapply semantics, parking does not block later actions, GC never sweeps a queued body, generation bump visibility, a store stamped with a newer version is refused.
- [x] Fold the delta into cairn/spec/store.md and log the change.
