---
cairn: log
change: statements-are-prepared-once
date: 2026-11-05
---

# Statements are prepared once per connection

Every canonical statement the client runs more than once now goes through rusqlite's statement cache: `PreparedCached::execute_cached` and `query_row_cached` (src/client.rs) beside `rows` and `bind_list`, which prepare through `prepare_cached` too, every connection set up by `prepare_connection` with a capacity of 256 (`STATEMENT_CACHE`). rusqlite's `cache` feature is enabled for it (hashlink, and foldhash, whose Zlib licence deny.toml now allows, as the apps' do), the crate having taken rusqlite without its defaults and so without any cache. A query compiled per search keeps `prepare` and `execute`, its text being new each time. Behaviour is unchanged; no capability spec moves.

Why: preparing a store write compiles the programs of the triggers it fires, which costs about as much again as running it (pimdir cairn/changes/performance-audit-2026-11-05, F4). tests/search_scale.rs, release build, one idle core, three runs each: 50,000 summary-only messages written through the owner in 17.55 s before, 2.94 s after (6x); the index built in 5.05 s before, 4.80 s after; the queries unchanged, their cost being the compiled query's.

Guarded by the whole suite unchanged (816 passed, `--all-features`); a cache is no behaviour a test can fail without.
