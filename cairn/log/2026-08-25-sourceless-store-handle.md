---
cairn: log
change: sourceless-store-handle
date: 2026-08-25
---

# An operation with no side no longer names one

`PimdirStore::open(dir, source)` bound every handle to a source, including the handles whose operations never read one. The store keeps two handles now: `PimdirStore`, which the constructors return and which carries the whole store-wide surface, and `PimdirSourceStore`, which `for_source(source)` yields and which carries the seam. Capabilities `store` and `cli` moved.

## What landed

- **The constructors lose the source** (capability `store`). `open(dir)`, `open_with_hash(dir, hash)` and `open_read_only(dir)` name no side; `for_source(source)` binds one, beside the existing `for_account(account)`, which either order now accepts. Breaking for every consumer.

- **The source-bound operations moved behind the source-bound handle**: the `ReplicaStorage` impl (`load`, `lookup_objects`, `write`), `write_rekeyed`, `drain_collection` and the staging behind it, plus the residual they share. A handle that named no source cannot reach them, and one that did cannot forget. The rest — every client read, retention and its purges, the queue's reads, `drop_action`, `fail_action`, the generations — stayed on `PimdirStore`, reachable through the source-bound handle by `Deref`.

- **`FALLBACK_SOURCE` is gone** (capability `cli`), with the two constructors that existed to invent a name: `read()` opened every inspection verb's handle as a source called `"pimdir"`, and `owner_any_source()` ran `SELECT DISTINCT source FROM bindings` — a scan of the second-largest table — to pick a name it then never read. `item purge`, `queue cancel`, `check` and every read open the source-less handle; `item restore` keeps `--source` and its multi-source refusal, and now refuses a store that has synced no source at all rather than creating the item for an invented one.

- **`distinct_sources` keeps three callers**: `store info` and `export`, where listing the sources is the answer the user asked for, and `item restore`'s no-flag branch, where the count is the question. The proposal expected two; the third is the refusal itself, which cannot be decided without asking.

## Tests

tests/sourceless.rs: a source-less handle reads a collection, cancels a queued action and purges what retention holds; and an operator pass over a store — read, purge, sweep, cancel — leaves `distinct_sources` empty, which is what the invented name was one write away from breaking. The existing suite carried the rest: 29 of its store opens no longer name a side, and the read-only test writes through a source-bound handle to prove the read-only refusal survives the split.
