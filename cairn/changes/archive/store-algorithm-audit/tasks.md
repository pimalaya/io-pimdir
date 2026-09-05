---
cairn: tasks
change: store-algorithm-audit
---

# Tasks

Triage first: each accepted item became its own change with its own delta. The list is reconciled with the log entry of 2026-08-25 and its same-day addendum; what later changes landed is credited to them.

## Correctness

- [x] Compare `sort_key` in `item_columns_eq`; add the update case to `tests/sort_key.rs`, verified failing first.
- [x] Replace `TOP_SORT_KEY` with a null-guarded descending cursor; test a key above the old sentinel.
- [x] Guard the queue drain with a delete-first `RETURNING id` (`CLAIM_ACTION`).
- [x] Fsync the parent directory after every blob rename.
- [x] Decode a malformed flag set as an error or as `Unknown`, never as known-empty.
- [x] `CHECK (refcount >= 0)` in the schema; sweep on `<= 0`. The sweep predicate landed here; the constraint landed with `engine-merge`, once the schema was the spec's own migration rather than a reconciled draft.
- [x] Write `created_at` through `strftime`, as `RETAIN_ITEM` already does.
- [x] Following pimdir: scope `lookup_objects` (by account, per the addendum, not by collection), encode base presence (`base_present`, per the addendum). ~~Settle what an unreferenced object means for the sweep.~~ Dropped: `manual-gc` made the store never collect itself, so a batch sweeps nothing.
- [x] Close whether io-replica produces a base with unknown flags, no revision and no object on a linked placement: closed by making presence a column of its own rather than an inference.

## Shape

- [x] Batch-scoped `load_hub`, with `bindings(collection, source, handle)` so a drop seeks instead of scanning. Benchmarked at 1k, 4k and 16k items; the probe is not kept as a test.
- [x] Answer the drain's duplicate check and handle lookup with point queries instead of hub loads (addendum).
- [ ] ~~Scope garbage collection to the hashes the batch decremented; skip it entirely when none were.~~ Dropped: `manual-gc` removed collection from the write path altogether.
- [x] Make the residual a map keyed by collection and handle. Whether it must survive a crash was answered by `engine-merge`: probes are rows, and the in-memory residual is gone.
- [ ] ~~Read `distinct_sources` from `sources`, or index `bindings(source)`.~~ Dropped: still open, and it needs a decision on whether the `sources` table alone is authoritative before either fix is right.
- [x] `items_by_seq_global`. ~~Plus a `placements_by_seq` read so the CLI's `locate` stops looping collections.~~ Dropped: the CLI's `locate` loops collections still, and a store holds few enough for that not to matter.
- [x] Write blobs before `BEGIN IMMEDIATE`. Landed with `store-write-path`.
- [x] One set-based statement for `release_pins` (addendum).

## Compaction

Landed by `store-compaction`, as its own change, except the last item.

- [x] One row-collecting helper across the fourteen call sites.
- [x] A macro for the statement table; drop both `include_str!` guard tests.
- [x] Collapse the three row structs into one, `PimdirPlacement` excepted.
- [x] Fold `PimdirDb`'s reads onto `PimdirStore`; one connection.
- [x] One `hash_algo`/`hash`/`hasher` implementation.
- [x] One `fail_action`; one delete statement for the queue.
- [x] Split `PimdirError::Version`.
- [ ] ~~Fix the `LOAD_ITEMS` claim in `tests/spec_drift.rs` and the O(changed rows) docstring.~~ Dropped: `vendored-spec-sql` rewrote tests/spec_drift.rs against the generated statements, and the claim went with the old suite.
