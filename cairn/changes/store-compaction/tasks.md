---
cairn: tasks
change: store-compaction
---

# Tasks

Land in this order, testing between each: every step is behaviour-preserving on its own, and a failing test means the step changed something.

- [ ] One `rows(stmt, params, map) -> Result<Vec<T>>` helper across the fourteen call sites.
- [ ] `macro_rules!` declaring each statement and building `sql::ALL`; delete the two `include_str!` guard tests it makes unnecessary.
- [ ] Collapse `PimdirItem` / `PimdirPlacement` / `PimdirRetainedItem` into one type with an optional retention and an optional placement; one row mapper. Breaking for anyone constructing them.
- [ ] Fold `PimdirDb`'s reads onto the store as a diagnostics block; `check` and `store info` use one connection.
- [ ] One `fail_action(id, Option<&str>)`; `park` becomes a call to it.
- [ ] Split `PimdirError::Version` into the two facts it carries.
- [ ] `cargo test` unchanged at every step, `cargo clippy --all-targets --all-features`, `cargo fmt`.
- [ ] CHANGELOG under `### Changed` (breaking: the row types and the error enum); log entry; mark landed.
