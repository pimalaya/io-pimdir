---
cairn: delta
change: action-queue
---

# Spec delta

## ADDED Requirements

### Requirement: Producers append, only the owner pops
The store SHALL support the pimdir action queue: any process may act as a producer whose sole write is the single enqueue transaction (ensure_collection, at most one object upsert pinning a pre-written blob, one queue insert). Only the owner SHALL read-and-remove queue rows: each pending action is applied to items and bindings and its row deleted in the same transaction, so application is exactly-once and never partially visible. Failing actions accumulate `attempts`; permanently failing actions are parked with `error` set, skipped without blocking later actions, queryable, and never silently deleted.

### Requirement: Queued bodies are pinned
An object referenced by a pending queue row's `object_hash` SHALL count as referenced under the incremental refcount scheme, so garbage collection never sweeps a body between enqueue and apply. The pin is taken at enqueue and released when the row is deleted, with the applied item's own reference taken in the same transaction.

### Requirement: Collection generation is the handle-space epoch
`collections.generation` SHALL start at 1 and be bumped only by the owner, in the same transaction as a handle-space rebuild (rekey). It SHALL be exposed on the read surface so frontends derive epoch-dependent protocol values (an IMAP UIDVALIDITY) from the store alone. Ordinary syncs, full resyncs from an expired checkpoint, and content changes never bump it.

### Requirement: Pending actions are readable
The read surface SHALL expose a collection's pending (non-parked) actions in append order, so a frontend can overlay them on its item projection for read-your-writes.

## MODIFIED Requirements

### Requirement: Schema version
The store schema is pimdir version 1 (`user_version` 1), including the `queue` table and `collections.generation`. A store stamped with a newer version is refused, never migrated: the spec is a draft, and draft stores are recreated.

## REMOVED Requirements

None.
