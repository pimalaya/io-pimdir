---
cairn: spec
capability: store
status: current
---

# Store

A pimdir store is a SQLite index (`pimdir.db`, `STRICT` tables, schema version in
`user_version`) plus a content-addressed blob directory. It implements
io-replica's storage seam (`load` / `lookup_objects` / `write`) for one source,
and is the portable, cross-implementation form of that seam (the pimdir spec).

### Requirement: A write batch is one transaction
`write` SHALL apply its `ReplicaWriteOp` batch as a single SQLite transaction:
object bytes are written to the blob file (temp → fsync → rename) before the row
that references them; placement upserts/drops fold into the collection's hub and
are saved; refcounts are recomputed; zero-refcount objects are collected, their
rows dropped inside the transaction and their blob files unlinked only after
commit. A crash SHALL leave at worst an orphan blob, never a row without its body.

### Requirement: Blobs are content-addressed and sharded
An object's bytes SHALL live at `objects/<hash[0:2]>/<hash[2:4]>/<hash>`,
immutable once written, so an identical body delivered twice is stored once.
`PimdirBlobs` reads a blob independently of the SQLite connection, so a body can
be read while the store is mutably borrowed to service a sync.

### Requirement: Several sources share one store
A store MAY be opened as several source handles (`"left"`, `"right"`, …) over the
same files; each services the seam for its own source, and the shared database is
the multi-source hub. `load_hub` reads a collection's whole hub (every source's
bindings) for a consumer that projects each side.

### Requirement: Collections declare a media type
A collection SHALL carry a `kind` (an IANA media type). `ensure_collection` sets
it; the lazy collection creation inside `write` uses `ON CONFLICT DO NOTHING` and
never clobbers a declared kind. This makes the store self-describing and lets one
store hold several item kinds.

### Requirement: A body may be ingested and emitted by streaming
The store SHALL be able to persist an object from a byte stream (`Read`),
computing its content hash incrementally, with the same temp → fsync → rename
durability as a buffered write, so a large body is never held whole; and it SHALL
expose a stored object as a readable stream for the same reason on the read side.

### Requirement: A byteless object write indexes an already-stored blob
A `StoreObject` carrying no bytes — its blob already persisted by a streaming
fetch under its content-addressed path — SHALL record the object row and refcount
without writing bytes. Refcounting and garbage collection are unchanged.

> Initial seed spec (Cairn adopted 2026-07-31): captures the store's core
> guarantees; further capabilities may be spelled out as they are touched.
