---
cairn: log
change: action-queue
date: 2026-08-07
---

# Action queue and collection generations

Implemented the pimdir action queue and collection generations (pimdir repo:
migrations/0001_init.sql, SPEC.md §14 and §15): the generic `queue` table plus
`collections.generation`, the write door for the multi-process split (one
owning sync process, frontends as readers plus producers).

**Schema.** `sql.rs` gains the `queue` table, its `queue_by_collection` index
and the `collections.generation` column, byte-consistent with the canonical
file (verified by diff); open() creates the whole version 1 schema in one
`BEGIN IMMEDIATE` transaction, keeping `user_version` and `store_meta.version`
in agreement (spec §4.2). A store stamped with a newer version is refused with
the new `PimdirError::Version` instead of being half-read: the spec is a
draft, and draft stores are recreated, never migrated.

**Producer.** New `PimdirProducer` (open + enqueue + pending_actions): opens the
existing database read-write without creating or migrating it (a version
mismatch errors; the owner opens first), and `enqueue` runs exactly the §14.1
transaction — `ensure_collection`, at most one `store_object` upsert when the
action references a body (the caller wrote the blob durably first via
`PimdirBlobs::writer` and passes its size), one queue insert, plus the
incremental +1 refcount pin on the body. It coexists with the single-writer
serialisation because the guard *is* the per-transaction `BEGIN IMMEDIATE` and
the 30s busy timeout: the producer's short append serialises against the owner's
batches on the same lock, exactly the coexistence §7 sanctions.

**Owner drain.** `PimdirStore` gains `queued_collections`, `pending_actions`
(decoded, append order — also the frontend's read-your-writes overlay),
`parked_actions`, and `drain_collection`. Each action applies and deletes its
row in one transaction; a permanently unappliable action (malformed payload,
unknown version or kind, unknown `seq`, duplicate `add` link id) parks with its
error and never blocks later actions; a transient failure bumps `attempts` and
stops the pass to preserve apply order.

**Drain implementation choice.** The drain resolves `seq` to the link id, finds
this source's projected placement, and pumps the corresponding **real
io-replica `ReplicaMutate` coroutine** (SetFlags/Remove/Move/Copy/Edit) fed a
hub projection loaded inside the drain transaction, then folds the yielded
write ops through the store's own machinery — the `write` transaction body was
refactored into a shared `apply_ops` + `collect_garbage` pair so the seam's
`write`, the drain and the rekey write all reuse one folding. This was chosen
over direct SQL folding because it keeps the mutation semantics (dirty /
tombstone / created staging, collision and conflict rules) the engine's, not a
re-implementation, while still committing the queue delete atomically with the
effects. Two deliberate deviations from a pure coroutine drive: `add` is staged
directly as the same `Created` placement `ReplicaMutate::Add` stages (the
mutation type demands body bytes the drain does not need to read back), and
`StoreObject` ops are stripped from mutation output, since the object row was
indexed (and its blob written) at enqueue and a re-store would clobber the
recorded size.

**Refcounts.** `queue.object_hash` joins the incremental scheme: +1 inside the
enqueue transaction, −1 as the drain deletes the row, with the applied item's
own reference taken by the same transaction's hub diff — the hand-over is
exact, and GC can never sweep a queued body (the reference `recompute_refcounts`
in pimdir/queries counts queue rows; this repo's specced substitution stays
incremental).

**Generation.** `PimdirCollection` gains `generation`; `PimdirStore::generation`
loads it and `write_rekeyed(collection, ops)` applies a rebuild batch and bumps
the epoch in the same transaction (io-replica's rekey has no marker op in its
write batch, so the owner routes rekey writes here instead of `write`).

**Codec.** `codec.rs` gains `PimdirAction` (the six v1 kinds), `kind()`,
`object_hash()`, `action_to_payload` / `action_from_payload` (versioned JSON,
leading `v`, strict decode with `PimdirActionError` — unlike the lenient column
decoders, since a malformed action must park, not decay).

Verified: 10 new queue integration tests (enqueue/drain round-trip per kind,
absolute + idempotent set-flags, idempotent remove, duplicate-add parking,
parking not blocking later actions, GC pinning with exact hand-over, generation
bump visibility across handles, refusal of a store stamped with a newer
version) plus 5 codec unit tests; full suite green, fmt + clippy clean on both
feature combos, embedded DDL diffed byte-consistent against the canonical
file.

Spec updated: `store` (ADDED: producers append / owner pops, queued bodies are
pinned, collection generation is the handle-space epoch, pending actions are
readable; ADDED as new: the schema-version requirement, version 1 including
the queue and generations, newer stamps refused).
