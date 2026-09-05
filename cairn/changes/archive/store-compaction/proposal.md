---
cairn: change
id: store-compaction
status: landed
created: 2026-08-25
---

# The same code, once

## Why

The cold-eye audit (`store-algorithm-audit`) counted around 460 lines that exist twice or more, roughly 15% of the crate, with no behaviour attached to the duplication. It was deliberately left out of that change: mixing a pure refactor into a batch of behaviour fixes makes the resulting history useless to bisect. The behaviour fixes have landed and carry regression tests, which is exactly the safety net a refactor wants.

## What

- **One row-collecting helper.** Fourteen functions each write `let mut v = Vec::new(); for row in rows { v.push(row?) } Ok(v)`: `list_collections`, `list_collections_by_account`, `list_accounts`, `link_placements`, `object_placements`, `list_items`, `sorted_page`, `list_retained`, `distinct_sources`, `queued_collections`, `parked_actions`, `collect_garbage`, `purge_retained_before`, `load_pending_actions`.
- **A macro for the statement table.** `sql::ALL` is a hand-written index of every constant, kept in sync by two guard tests that `include_str!("sql.rs")` and re-parse it. One `macro_rules!` declaring each statement and building the index in the same expansion makes both guards unnecessary.
- **One row struct.** `PimdirItem`, `PimdirPlacement` and `PimdirRetainedItem` are near-identical, and their columns are typed differently for no reason (`String` against `ReplicaLinkId`, `Option<String>` against `Option<ReplicaHash>`). One struct with an optional retention and an optional placement, and one row mapper instead of four.
- **One connection.** `PimdirDb` opens a second read-only connection and re-implements reads the library already has, while `store info` and `check` hold both at once. Its statements are plain selects and belong on the store as a diagnostics block.
- **One failure path.** `park`, `fail_action(Some)` and `fail_action(None)` are three ways to record the same thing.
- **`PimdirError::Version`** conflates "this store is newer than this crate" with "a producer opened a store that does not exist yet", as its own doc admits. Two variants.

## Scope / non-goals

- No behaviour change anywhere. Every existing test must pass untouched; a test that needs editing is a signal the refactor changed something.
- Not included: `reconcile_draft_shape` and the `ENSURE_*` statements, which exist only because version 1 is edited in place. They go when the schema freezes, not before.
