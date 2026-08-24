---
cairn: tasks
change: store-spec-conformance
---

# Tasks

- [x] Reconcile `items.sort_key` and `items_by_sort` on open, beside the other folded-in columns.
- [x] Compare `PRAGMA user_version` with `store_meta.version` on both opens; refuse a disagreement.
- [x] Encode an unknown flag set as `NULL`, and a queue payload's as `null`.
- [x] Refuse a store whose foreign keys carry no `ON UPDATE CASCADE`, on both opens.
- [x] Tests: an earlier-draft store derived from the current schema reopens; a store derived from it without its cascades is refused; disagreeing stamps are refused; `NULL` and `'[]'` round-trip apart.
- [x] CHANGELOG.
- [x] Fold `delta.md`; log; land.
