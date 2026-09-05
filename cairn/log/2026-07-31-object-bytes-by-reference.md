---
cairn: log
change: object-bytes-by-reference
date: 2026-07-31
---

# Object bytes by reference (streaming blob I/O)

Added the streaming blob path so a large body is never held whole. `PimdirBlobs`
gained `writer()` → `PimdirBlobWriter` (a `Write` sink over a temp file that, on
`commit(hash)`, fsyncs and renames into the sharded content-addressed path, or
drops the temp when the body already exists) and `reader(hash)` → `Option<File>`
(the append side; the file's metadata gives the octet length IMAP `APPEND` needs
up front). A per-write atomic counter names the temp file so concurrent writers
of one store do not collide. `write` now accepts a byteless `StoreObject` (its
blob was streamed into place during the fetch): it records the object row and
refcount, writing no bytes. `busy_timeout` was already set for multi-handle use.

Two round-trip tests added: a body streams in (in chunks) and back out with a
matching hash; a byteless `StoreObject` indexes an already-streamed blob, which
survives reopen and is not GC'd while a placement references it. All store tests
and the codec tests pass; fmt clean.

Depends on io-replica `object-bytes-by-reference` (the `Option<Vec<u8>>`
`StoreObject` body).

Spec updated: `store` (ADDED: a body may be ingested and emitted by streaming; a
byteless object write indexes an already-stored blob).
