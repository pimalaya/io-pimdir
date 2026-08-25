---
cairn: tasks
change: store-algorithm-audit
---

# Tasks

Triage first: each accepted item becomes its own change with its own delta. Nothing below is landed by this change.

## Correctness

- [ ] Compare `sort_key` in `item_columns_eq`; add the update case to `tests/sort_key.rs`, verified failing first.
- [ ] Replace `TOP_SORT_KEY` with a null-guarded descending cursor; test a key above the old sentinel.
- [ ] Guard the queue drain with a delete-first `RETURNING id`.
- [ ] Fsync the parent directory after every blob rename.
- [ ] Decode a malformed flag set as an error or as `Unknown`, never as known-empty.
- [ ] `CHECK (refcount >= 0)` in the schema; sweep on `<= 0`.
- [ ] Write `created_at` through `strftime`, as `RETAIN_ITEM` already does.
- [ ] Following pimdir: scope `lookup_objects` by collection, encode base presence, settle what an unreferenced object means for the sweep.
- [ ] Close whether io-replica produces a base with unknown flags, no revision and no object on a linked placement.

## Shape

- [ ] Batch-scoped `load_hub`, with `bindings(collection, source, handle)` so a drop seeks instead of scanning. Benchmark a single-item write at 20k and 40k items before and after.
- [ ] Answer the drain's duplicate check and handle lookup with point queries instead of hub loads.
- [ ] Scope garbage collection to the hashes the batch decremented; skip it entirely when none were.
- [ ] Make the residual a map keyed by collection and handle; decide whether it must survive a crash.
- [ ] Read `distinct_sources` from `sources`, or index `bindings(source)`.
- [ ] `items_by_seq_global`, plus a `placements_by_seq` read so the CLI's `locate` stops looping collections.
- [ ] Write blobs before `BEGIN IMMEDIATE`.
- [ ] One set-based statement for `release_pins`.

## Compaction

- [ ] One row-collecting helper across the fourteen call sites.
- [ ] A macro for the statement table; drop both `include_str!` guard tests.
- [ ] Collapse the three row structs into one.
- [ ] Fold `PimdirDb`'s reads onto `PimdirStore`; one connection.
- [ ] One `hash_algo`/`hash`/`hasher` implementation.
- [ ] One `fail_action`; one delete statement for the queue.
- [ ] Split `PimdirError::Version`.
- [ ] Fix the `LOAD_ITEMS` claim in `tests/spec_drift.rs` and the O(changed rows) docstring.
