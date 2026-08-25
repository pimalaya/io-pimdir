---
cairn: tasks
change: store-compaction
---

# Tasks

Land in this order, testing between each: every step is behaviour-preserving on its own, and a failing test means the step changed something.

- [x] One `rows(stmt, params, map) -> Result<Vec<T>>` helper across the fourteen call sites.
- [x] `macro_rules!` declaring each statement and building `sql::ALL`; delete the two `include_str!` guard tests it makes unnecessary.
- [x] Collapse `PimdirRetainedItem` into `PimdirItem` with an optional `retention`; one row mapper for both. **Not** `PimdirPlacement`: the two placement statements are a narrower projection, with a collection and an account and without `meta` or `sort_key`, so folding it in would have meant widening the reads or filling those fields with `None` and `""`. It keeps its shape and gains the typing the others had (`ReplicaLinkId`, `ReplicaFlags`, `ReplicaLevel`). Breaking either way.
- [x] Fold `PimdirDb`'s reads onto the store as a diagnostics block (src/client/diagnostics.rs, statements in `sql`); `check` and `store info` use one connection. Their types derive `Serialize` behind a new off-by-default `serde` feature the `cli` feature turns on, since `--json` renders them and CLI-side mirror structs would be the duplication this removes. `cairn/spec/cli.md` loses the requirement that let those reads bypass the library.
- [x] One `fail_action(id, Option<&str>)`; `park` becomes a call to it.
- [x] Split `PimdirError::Version` into the two facts it carries.
- [x] `cargo test` unchanged at every step but the type collapse, where tests/retention.rs reads the fields that moved; no assertion changed its meaning. `cargo clippy --all-targets --all-features`, `cargo fmt`.
- [x] CHANGELOG under `### Changed` (breaking: the row types and the error enum); log entry; mark landed.
