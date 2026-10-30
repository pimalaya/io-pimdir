---
cairn: tasks
change: a-search-answers-at-scale
---

- [x] Benchmark: tests/search_scale.rs, 50,000 summary-only messages through the owner, index build, a page and a count per query, query plans with `PIMDIR_PLAN`
- [x] Profile: EXPLAIN QUERY PLAN of the compiled `is:unread` and of the extraction's summary reads; timings at 5,000 and 20,000 to show the square
- [x] src/client/search/compile.rs: `member`, every term's membership never `NULL`
- [x] src/client/search.rs: extraction through `get_*` by seq and `load_addresses`, seeks
- [x] src/client/search/compile.rs: the title read for the `name` order alone
- [x] Every search vector green (tests/search.rs `every_search_vector_answers`, `every_field_alone_answers_in_every_order`)
- [x] CHANGELOG; spec search.md folded; log
