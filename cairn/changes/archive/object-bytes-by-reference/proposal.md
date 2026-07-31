---
cairn: change
id: object-bytes-by-reference
status: landed
created: 2026-07-31
---

# Object bytes by reference (streaming blob I/O)

## Why

The storage side of bounded-memory body transfer. Today `write` receives an
object's whole body as a `Vec<u8>` (`StoreObject.body`) and writes it in one
shot, and a body is read back whole for an append. For a large message that is a
full-size allocation on each side. This change lets the store ingest and emit a
body as a **stream**, and index an object whose bytes a streaming fetch already
persisted.

Paired with io-replica's [`object-bytes-by-reference`] (the `StoreObject` /
fetched-body shape change) and neverest's `object-bytes-by-reference` (the
streaming remote). This repo's part is the blob I/O.

## What

- Persist an object from a `Read`: temp file in the shard dir, hash computed
  incrementally as bytes flow, `fsync`, rename into the content-addressed path —
  same durability ordering as the buffered write, but never holding the body.
- Expose an object as a readable stream (`PimdirBlobs`) for the append side.
- Accept a **byteless** `StoreObject` (the blob was already persisted by a
  streaming fetch): record the object row and refcount, write no bytes.

## Scope / non-goals

- Depends on io-replica `object-bytes-by-reference` for the byteless-`StoreObject`
  shape.
- No change to the schema, the transaction discipline, or the sharded layout.
