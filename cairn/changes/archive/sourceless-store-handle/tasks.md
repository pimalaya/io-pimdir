---
cairn: tasks
change: sourceless-store-handle
---

# Tasks

- [x] `PimdirStore::open(dir)` and `open_read_only(dir)` drop the source parameter; add `for_source(source)` beside `for_account(account)`.
- [x] Move the source-bound operations behind the source-bound handle: `load`, `write`, `write_rekeyed`, `drain_collection`, `apply_queued`, `stage_action`. A source-less handle must not reach them.
- [x] Keep source-less: `purge`, `purge_retained_before`, `revive_item`, `drop_action`, `pending_actions`, `parked_actions`, `queued_collections`, and every client read.
- [x] Delete `FALLBACK_SOURCE` and the two owner constructors that existed to invent one (`owner_any_source`, and `owner`'s no-flag branch keeps only the multi-source refusal for `item restore`).
- [x] CLI: `item purge`, `queue cancel`, `check`, and every read open a source-less handle. `item restore` and any drain keep `--source`.
- [x] `distinct_sources` keeps three callers, not the two expected: `store info` and `export`, plus `item restore`'s no-flag branch, whose refusal cannot be decided without the count.
- [x] Tests: a source-less handle purges, cancels and reads; no path constructs a store with an invented source name. `item restore`'s refusal is unverified: the repository has no CLI harness, and it now refuses a source-less store too.
- [x] `cargo test`, `cargo clippy --all-targets --all-features`, `cargo fmt`.
- [x] CHANGELOG under `### Changed` (breaking: the constructor loses a parameter); fold `delta.md` into `cairn/spec/store.md`; log entry; mark landed.
- [x] Hand over to `single-owner-lock`, which acquires on the same constructors.
