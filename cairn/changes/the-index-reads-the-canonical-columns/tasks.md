---
cairn: tasks
change: the-index-reads-the-canonical-columns
---

- [x] spec/ re-vendored from pimdir; tests/spec_drift.rs green
- [x] `has:attachment` through `attached`; `insert_object` binds `:attachment`; `pimdir_column_size` removed
- [x] `behind` through the canonical statement
- [x] `store_reconciled` through `set_index_cursor` and `load_index_meta`; `index_build` and its statements removed
- [x] The index shape test: `open` rebuilds, `open_query` refuses an index whose shape differs; test failing first (with the shape check removed: the index opened for queries)
- [x] Search vectors green
- [x] Spec: search.md folded; log; CHANGELOG
