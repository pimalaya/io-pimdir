---
cairn: log
change: as-fast-as-notmuch
landed: 2026-11-21
---

# An intersection pages from its bitmaps

The 1M tier (results-2026-11-21.md, "The 1M tier") put the page of `power california` at 337 ms against notmuch's 17: the plan walked the store's order across 6,622 folders, probing each row against both terms, and met the 29k hits' first page late.

- **The page** (src/client/search.rs `bitmap_page`, src/client/search/compile.rs `met_rows`): a query of text terms alone over a current index whose root is an `and` of two positive text terms or more, one set not capped, reads its hits from the bitmaps a count builds (`bitwise_hits`, the joint match and the split correction), writes them to `temp.hit_page`, and cuts the page with one statement joining the store, as `set_page` cuts a set's, its cursor bound by `page_params`.
- **The gate**: a capped count of the documents the terms match together (`JOINT_BODIES`, `JOINT_SUMMARIES`) above `PAGE_HITS_MAX` (40,000), or hits above it, keeps the walk. A seek per hit costs about 2.5 µs: a first version reading every text query this way made `subject:meeting` 38 to 105 ms, `"natural gas"` 40 to 122 and `california or texas` 45 to 95, and without the gate `enron california` (78k hits) 240 to 279.

## Proof

A page answering no rows this way (`met_rows` with a false condition) fails two cases of tests/search.rs; tests/search_plan.rs sees no statement scanning the store's items.

## Measured

1M tier, core 12, hyperfine means of 10, ms, io-pimdir 509fd83 then the same with this change, pages of 50 equal: `power california` 337 / 112, `california texas` 87 / 72, `enron california` 239 / 243, `california not gas` 54 / 54, `deriv*` 106 / 102, `subject:meeting` 34 / 36, `kiwi` 9.0 / 8.8.
