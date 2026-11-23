---
cairn: tasks
change: an-io-free-indexer
---

# Tasks

- [ ] Review with the user: the yields, the ops, queries compiled per shape (open: the yields grew from the four proposed to ten, see cairn/log/2026-10-31-an-io-free-indexer.md)
- [x] Feature split: `search` without `client`; extraction, tags, threads and occurrences under src/search/, building with `--no-default-features --features search`
- [x] `PimdirIndexRefresh` coroutine and `PimdirIndexOp`; folded text in the ops; `pimdir_fold` gone
- [x] `PimdirIndex::refresh` driving the coroutine; the existing search tests green unchanged
- [x] A test driving the coroutine over an in-memory consumer, against the canonical driver's index
- [x] Store on draft-07: flag tables, `temp.param`, `target_collection`, `store_meta.shape`, the draft-06 migration; a test over a draft-06 store failing first
- [x] `cargo fmt`, clippy, both feature builds; delta folded into spec/search.md and spec/store.md; log
