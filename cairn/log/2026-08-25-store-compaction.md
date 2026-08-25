---
cairn: log
change: store-compaction
date: 2026-08-25
---

# The same code, once

The cold-eye audit counted around 460 lines that existed twice or more, roughly 15% of the crate, with no behaviour attached to the duplication. It was left out of `store-algorithm-audit` deliberately: mixing a pure refactor into a batch of behaviour fixes makes the history useless to bisect. Those fixes have landed and carry regression tests, which is the safety net this wanted.

## What landed

- **One row-collecting helper.** Fourteen reads each wrote `let mut v = Vec::new(); for row in rows { v.push(row?) } Ok(v)`. They call `rows(conn, sql, params, map)` now, which is `prepare().query_map()?.collect()`. A `Transaction` derefs to a `Connection`, so the reads inside a write batch use it too. `read_hub_items` and `read_hub_bindings` went with it: they existed to avoid writing the prepare-and-bind twice across the scoped and unscoped arms of one match, and one binding list built with an optional `:links` replaces both.

- **A macro for the statement table.** `sql::ALL` was a hand-written index of every constant, kept honest by two tests that `include_str!`ed the module's own source and re-parsed it. One `macro_rules!` declares each statement and builds the index in the same expansion, so the gap is closed rather than watched. The invocation is deliberately not indented: half the statements are raw strings holding the spec's SQL verbatim, and indenting would rewrite that text.

- **One item type.** `PimdirRetainedItem` is gone; `PimdirItem` carries `retention: Option<PimdirRetention>` (the stamp, the source that retired it, the body's size). The two statements select the same seven columns, the retained one plus three, so one row mapper reads both and takes the extras when the row has them. In the CLI, `FoundItem`'s live-or-retained split collapses into asking the item whether it has a retention.

- **`PimdirPlacement` keeps its own shape** and gains the typing the others had: `ReplicaLinkId`, `ReplicaFlags`, `ReplicaLevel` instead of `String`, `Option<String>`, `i64`.

- **One connection.** `PimdirDb`, the operator tool's second read-only handle, is folded onto the store as `src/client/diagnostics.rs`, with its statements inlined beside the rest in `sql`. `store info` and `check` each held two connections to one file; they hold one.

- **One failure path.** `park` was a third way to write what `fail_action(id, Some(error))` already wrote; it is a call to it now, at the cost of one point read of the row's attempt count.

- **`PimdirError::Version` splits.** It conflated "this store's schema is not one this crate services" with "no owner has created this store yet", as its own doc admitted. An unstamped database is `Uncreated` now, which is the one a producer or a reader can actually act on: wait for the owner.

## The claim about tests

The proposal said every existing test must pass untouched, and that a test needing an edit is a signal the refactor changed something. That held for every step except the type collapse, which the same proposal calls breaking: tests/retention.rs reads the fields that moved, and the reads move with them. No assertion changed its meaning, and no test was deleted or weakened.

## Deviations

- **`PimdirPlacement` was not folded in.** The proposal grouped it with the other two as near-identical. It is not: `LIST_LINK_PLACEMENTS` and `LIST_OBJECT_PLACEMENTS` are a narrower projection with a collection and an account and *without* `meta` or `sort_key`. Folding it in would have meant either widening those statements or filling the fields with `None` and `""`, and a type that says "this item has no meta" where the read never asked is worse than two types.

- **A `serde` feature.** The diagnostics types are rendered by `--json`, so they derive `Serialize` behind a new off-by-default feature that `cli` turns on. The alternative was a set of CLI-side mirror structs, which is the duplication this change exists to remove.
