---
cairn: tasks
change: object-bytes-by-reference
---

# Tasks

- [x] Streaming blob write: store an object from a `Read` — temp file in the
      shard dir, incremental hash, `fsync`, rename into the content-addressed
      path. Same durability ordering as the buffered path.
- [x] `PimdirBlobs`: open an object as a `Read` for the append side.
- [x] `write`: handle a byteless `StoreObject` — index the object row and
      refcount it, skip the blob write (bytes already persisted by the fetch).
- [x] Tests: stream a large body in and back out with a matching hash; a byteless
      `StoreObject` indexes an already-present blob and refcounts/GCs correctly.
- [x] Fold spec: `store`. Log entry.

Every task above landed on 2026-07-31 (see the log entry: `writer()`, `reader()`, the byteless `StoreObject` write and the two round-trip tests); the boxes were ticked on 2026-09-04 from that entry, the log being the truth.
