---
cairn: change
id: action-queue
status: landed
created: 2026-08-07
---

# Action queue and collection generations

## Why

The pimdir format gained the action queue and collection generations (pimdir repo: migrations/0001_init.sql, SPEC.md §14 and §15): a generic `queue` table plus a `collections.generation` column. The driver is the multi-process architecture decided on 2026-08-07: one owner process (the sync layer, neverest) is the store's only writer, while frontends (an IMAP or SMTP connector) are readers plus **producers** that request mutations by appending actions. Append-only for producers, pop-and-apply for the owner: this dissolves the read-modify-write concurrency problem without optimistic locking, because applying an action is a pure store mutation (the remote push happens later from the dirty state), so apply-plus-delete commits in one transaction, exactly-once. The generation column carries the handle-space epoch (a rekey after an IMAP UIDVALIDITY change) to readers, so a frontend derives its advertised UIDVALIDITY from the store alone.

## What

Implement them in io-pimdir:

- The `queue` table and the `collections.generation` column in the `sql` module's schema, byte-consistent with the canonical DDL.
- Producer surface: an enqueue API restricted to the §14.1 transaction (`ensure_collection`, optional object upsert pinning a pre-written blob, one queue insert). Producers never touch other tables; the existing single-writer guard stays owner-only, and the enqueue transaction uses its own short `BEGIN IMMEDIATE`.
- Owner surface: a drain API listing queued collections, loading a collection's pending actions in append order, applying each through the existing write path and deleting the row in the same transaction, incrementing `attempts` on failure, parking (setting `error`) on permanent failure. Parked actions are queryable, never silently deleted.
- Action codec: the six v1 kinds (`add`, `set-flags`, `remove`, `move`, `copy`, `update`) with their §14.3 JSON payloads, addressing existing items by public `seq`, in the no_std codec layer. The kinds mirror io-replica's mutation vocabulary on purpose: the queue is the cross-process projection of the engine's mutate verb, the drain applies each action by resolving `seq` to the internal identity and running the corresponding mutation through the existing write path, and an in-process consumer that owns its store (a mobile app, a CLI) keeps calling mutate directly with no queue in between.
- Refcount integration: a queue row's `object_hash` pins the object under the incremental refcount scheme (+1 at enqueue; the delete at apply releases it while the applied item row takes its own reference in the same transaction).
- Generation: `bump_generation` in the same transaction as a handle-space rebuild, `load_generation` on the read surface (PimdirCollection gains the field).
- Read-your-writes: expose pending actions of a collection on the read surface so a frontend can overlay them.

Out of scope here: the neverest drain loop, the connector producers, and any pimgate re-partition; those are their own repos' changes once this lands.
