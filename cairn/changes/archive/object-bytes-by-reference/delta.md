---
cairn: delta
change: object-bytes-by-reference
---

## ADDED Requirements

### Requirement: A body may be ingested and emitted by streaming
The store SHALL be able to persist an object from a byte stream (`Read`),
computing its content hash incrementally, with the same temp → fsync → rename
durability as a buffered write, so a large body is never held whole; and it SHALL
expose a stored object as a readable stream for the same reason on the read side.

### Requirement: A byteless object write indexes an already-stored blob
A `StoreObject` carrying no bytes — its blob already persisted by a streaming
fetch under its content-addressed path — SHALL record the object row and refcount
without writing bytes. Refcounting and garbage collection are unchanged.

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
