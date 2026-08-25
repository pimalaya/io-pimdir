---
cairn: change
id: store-algorithm-audit
status: landed
created: 2026-08-25
---

# What a first reader finds wrong in the store

## Why

This crate grew one change at a time, each reviewed against the state before it and none against the whole. On 2026-08-25 the spec, the migrations, the canonical queries and every file under `src/` were read cold, in full, by a reader with no history of the design. This file records the implementation-side findings. The ones that belong to the format itself live in pimdir under `spec-algorithm-audit`; the engine-side ones in io-replica under `engine-algorithm-audit`.

Nothing here is agreed. It is a triage list: each item that survives review becomes its own change with its own delta.

## What

### Correctness

**A sort-key-only change is discarded.** `item_columns_eq` (`src/client.rs:2321`) compares flags, object, meta, level, deleted, conflicted and conflict object, and omits `sort_key`, which `UPDATE_ITEM` writes. A placement arriving with a new key and nothing else changed issues no `UPDATE`, and the key is dropped. It bites whenever a connector fixes its derivation, a tzdb update moves a zoned start, or the second source of a two-source sync writes its own key, which §13 explicitly anticipates. `tests/sort_key.rs` covers preservation and never update, which is why the suite is green. One line.

**Descending pages hide items permanently.** `TOP_SORT_KEY = "\u{10FFFF}"` (`src/client.rs:261`) stands in for "no cursor" in `(sort_key, seq) < (:after_key, :after_seq)`. Sort keys are arbitrary writer-supplied text, and `"\u{10FFFF}\u{10FFFF}"` sorts above the sentinel, so such an item is invisible to every descending page while `count_items` still counts it. The statement should say what it means: `(:after_key IS NULL OR (sort_key, seq) < (...))`, binding SQL `NULL`. Same index plan, no unrepresentable key. Ascending is fine, since `''` really is the minimum.

**`lookup_objects` is not collection-scoped.** `LOOKUP_OBJECTS` (`src/sql.rs:491`) selects store-wide on `link_id` and the rows fold into a map where the last wins, so two accounts holding the same vCard `UID` hand each other's bodies across. Faithful to §14, and unsound; see the pimdir change.

**The queue drain can double-apply.** `load_pending_actions` (`src/client.rs:1126`) reads the pending rows outside any transaction; `apply_queued` then opens its own transaction per row and ends with `DELETE FROM queue WHERE id`, never checking the row is still there. Two owner handles are easy to get, since `pimdir item restore` opens one and drains while a sync holds another. `set-flags` is idempotent by design, `copy` and `add` are not. Making the delete the first statement of the transaction, `RETURNING id`, and rolling back when it returns nothing, makes exactly-once structural instead of conventional. The §8 advisory lock is separately unimplemented.

**A base of unknown flags, no revision and no object round-trips to no base.** Written as three nullable columns (`src/client.rs:2478`) and reconstructed only if one of them is non-null (`src/client.rs:2648`), so an agreed placement reads back as never-agreed, projects Dirty and re-pushes on every run. Faithful to §13, and §13 is the hole. Whether the engine currently produces that shape on a *linked* placement is unverified and worth closing against io-replica's `pull_flags`, which fabricates a base with no object while the placement holds a body.

**Blob renames are never durable.** `write_blob` (`src/client.rs:2693`) and `PimdirBlobWriter::commit` (`src/client.rs:1749`) both create, write, `sync_all` and rename, and neither fsyncs the parent directory. The SQLite commit is durable, so a power cut can leave a committed row pointing at a body that never landed, which is the one asymmetry §14 step 5 exists to prevent.

**An object with no same-batch referrer is swept at the end of that batch.** `STORE_OBJECT` inserts at refcount 0 (`src/client.rs:1904`) and `collect_garbage` (`src/client.rs:1974`) deletes every refcount-0 row and unlinks its blob after the commit. A consumer that streams bodies in one batch and places them in a later one loses them, silently. `tests/conflict.rs:56` records the behaviour as known. The format invites the pattern, so the fix belongs to both sides.

**Malformed flags JSON decays to a known-empty set.** `codec.rs:36` uses `unwrap_or_default()`, which yields `Known([])`, an authoritative "this item has no markers" that `absorb` then merges and the next write persists. A read-time decode failure is amplified into permanent loss. Error out, or decay to `Unknown`, which holds no opinion and cannot erase another source's flags.

**`refcount` has no floor.** Maintained only as `refcount + :delta`, with the sweep testing `= 0`, so a double release drives it negative and leaks the object forever without ever being reported as garbage.

**`created_at` is epoch milliseconds** (`src/client.rs:2019`), and the empty string when the clock is before the epoch, where the schema comment and §4.3 both say RFC 3339.

### Shape

**Every write reads and clones the whole collection.** `apply_ops` loads the hub (`src/client.rs:1967`), clones it, absorbs and diffs. Measured on release, tmpfs: 90 ms for a single-item write at 20k items, 156 ms at 40k, perfectly linear, and 2.4 ms with one item and 40k objects in the store, which proves the cost is the hub load rather than the object scan. Roughly two seconds per flag toggle on a 500k mailbox, against §1's promise of hundreds of thousands of items. The docstring at `src/client.rs:1896` claims O(changed rows), which is true of the writes and false of the reads that dominate.

The diff exists to avoid deleting and reinserting a collection, and pays a full read to compute what the caller already knows. Load only the link ids the batch touches. `absorb` needs the rows for the batch's placements plus, for a drop, a lookup by `(collection, source, handle)`, so one index turns io-replica's `absorb_drop` scan into a seek and `load_hub` into `WHERE collection = ? AND link_id IN (json_each(?))`. The whole write becomes O(batch).

**Draining N actions costs N collection loads, 2N for anything but `Add`.** The duplicate check (`src/client.rs:1375`) and the handle lookup (`src/client.rs:1435`) each load a hub to answer a point question that a primary key answers directly, on top of the load inside `apply_ops`. Fifty queued flag changes on a 100k mailbox read fifteen million rows.

**Garbage collection full-scans `objects` twice per transaction.** `LIST_GARBAGE_SIZED` and `DELETE_GARBAGE_OBJECTS` filter on `refcount = 0` with no index, and run unconditionally in `write`, `write_rekeyed`, `apply_queued` (once per action), `drop_action`, `purge` and `purge_retained_before`, including on batches that touched no object, which §14 already permits skipping. Scope it to the hashes the batch decremented, which `adjust_refcounts` already knows.

**The residual is a `Vec` scanned linearly.** `src/client.rs:1948` finds by position on every upsert, `drop_residual` retains over the same vector, and `lookup_objects` scans it again. An IMAP first sync probes the whole mailbox before linking it, so the residual grows to n while each insertion scans it: around 5x10^9 comparisons at 100k probes. A `HashMap<(collection, handle), placement>` makes every operation constant and shrinks the code. It is also process-local and unpersisted, so a crash mid-sync loses every probe, and two handles of one source hold different residuals.

**`distinct_sources()` full-scans `bindings`** with a temp b-tree for the distinct, and the CLI calls it on `item restore`, `item purge`, `queue cancel` and `store info` whenever `--source` is absent. `sources` already holds the pairs; the stated reason for preferring `bindings`, that a source appears there before its first checkpoint, costs one `UNION`, not a scan of the largest table.

**A store-global `seq` has no store-global index**, so `pimdir item show 42` loops every collection at two queries each, and any consumer resolving a bare `seq` scans.

**Blob I/O happens inside `BEGIN IMMEDIATE`** (`src/client.rs:1904`), so a multi-megabyte body serialises every other writer behind its `write_all` and `sync_all`. Blobs are content-addressed and immutable, so writing before the transaction is safe: a crash leaves an orphan file, which the check already handles.

**`release_pins` issues one `UPDATE` per hash** (`src/client.rs:984`), so purging 50k retained items runs 100k point updates in one transaction where one set-based statement would do.

## Compaction

Around 460 lines go away with no behaviour change, roughly 15% of the crate, plus another 110 once the schema freezes.

- Eleven copies of the same collect-rows-into-a-vec loop, across `list_collections`, `list_collections_by_account`, `list_accounts`, `link_placements`, `object_placements`, `list_items`, `sorted_page`, `list_retained`, `distinct_sources`, `queued_collections`, `parked_actions`, `collect_garbage`, `purge_retained_before` and `load_pending_actions`, become one helper. About 60 lines.
- `sql::ALL`, a hand-written 66-entry index, plus the two guard tests that `include_str!("sql.rs")` and re-parse it to check the index is complete, become one macro that declares each statement and builds the index in the same expansion. Both guards stop being necessary. About 130 lines.
- `PimdirItem`, `PimdirPlacement` and `PimdirRetainedItem` are three near-identical row structs whose columns are gratuitously typed differently, `String` against `ReplicaLinkId`, `Option<String>` against `Option<ReplicaHash>`. One struct with an optional retention and an optional placement, and one row mapper instead of four. About 90 lines.
- `PimdirDb` opens a second read-only connection and re-implements reads the library already has, while `store info` and `check` hold both at once. Its statements are plain selects and belong on `PimdirStore` as a diagnostics block. About 120 lines.
- `hash_algo`, `hash` and `hasher` are copy-pasted across `PimdirStore`, `PimdirProducer` and `PimdirBlobs`. About 40 lines.
- Three ways to record a queue failure, `park`, `fail_action(Some)` and `fail_action(None)`, are one method with an optional reason. About 20 lines.
- `DELETE_ACTION` and `CANCEL_ACTION` are byte-identical strings under two names, which a test comment already admits. The intent distinction belongs in the calling method's name.
- `reconcile_draft_shape`, the three `ENSURE_*_INDEX` statements, `check_rename_cascades` and `PimdirError::Unreconcilable` exist only because version 1 is edited in place. About 110 lines the day it freezes.
- `PimdirError::Version` conflates a store newer than the crate with a producer opening a store that does not exist yet, as its own doc says.

Two stale claims to fix while there: `tests/spec_drift.rs:23` says `LOAD_ITEMS` omits `sort_key`, which is false in both repositories, and the O(changed rows) docstring above.

## Scope / non-goals

- This change lands no edit. Accepted findings each get their own change, delta and log entry.
- Format-side findings are not repeated here; see `spec-algorithm-audit` in pimdir.
- The concurrency envelope has to be decided before the drain guard and the advisory lock can be: this crate supports several handles of one store, the CLI opens a second owner handle routinely, and §8 says one owner.
