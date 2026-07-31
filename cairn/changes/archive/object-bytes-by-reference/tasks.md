---
cairn: tasks
change: object-bytes-by-reference
---

# Tasks

- [ ] Streaming blob write: store an object from a `Read` — temp file in the
      shard dir, incremental hash, `fsync`, rename into the content-addressed
      path. Same durability ordering as the buffered path.
- [ ] `PimdirBlobs`: open an object as a `Read` for the append side.
- [ ] `write`: handle a byteless `StoreObject` — index the object row and
      refcount it, skip the blob write (bytes already persisted by the fetch).
- [ ] Tests: stream a large body in and back out with a matching hash; a byteless
      `StoreObject` indexes an already-present blob and refcounts/GCs correctly.
- [ ] Fold spec: `store`. Log entry.
