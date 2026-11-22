# Round 4 (2026-11-22): the owner's decisions, the per-hit cost

Owner decisions at the start (round-3.md end): the sources index yes, the `from` posting list yes as io-pimdir's own, the page presentation unchanged. Bench as round 3: 1M store, core 12, one hyperfine run per measure holding both commands. Scripts in .bench-notmuch/scripts: ab.sh A_BIN A_STORE B_BIN B_STORE (two pimdir builds head to head), vs.sh (PST=store overrides the store), cmp.sh (ST1, ST2 override each side's store).

## The coverage roll seeks (pimdir f55a31d, storage-draft-16)

A covering index on `sources(covered_at, covered_since, covered_until)` gave nothing: 56 index pages against 121 table pages, the same 0.42 ms in SQLite's shell, the cost being the 6,622 rows aggregated, not the pages. Built instead: three partial indexes, `sources_by_since` and `sources_by_until` over the sources that closed a round and hold that bound, `sources_filling` over those that never closed one (empty at rest), and the canonical `covered_all`, three scalar subqueries the min/max optimisation answers from an index end: 24 µs against 525 µs. The indexes are schema, so storage-draft-16 (0.19 released draft-15 stores, refused now as an earlier engine's).

The bench store copy runs/x2d/pd/store is x2c migrated by hand (the three indexes, `format_version` set to storage-draft-16; objects/ hard-linked); x2c stays for draft-15 binaries. index.hits is keyed by index.db's inode, so a copy rebuilds it on its first refresh (count --refresh-timeout 600s).

new9 (x2c) vs new10 (x2d), ms, count / page / threads:

| query | count | page | threads |
| --- | --- | --- | --- |
| all mail | 7.6 vs 7.1 | 16.4 vs 12.4 | 13.7 vs 13.2 |
| unread | 7.8 vs 7.4 | 10.6 vs 10.2 | 12.8 vs 12.4 |
| from name | 6 vs 5.3 | 8.4 vs 8 | 9.9 vs 9.4 |
| rare word | 3.1 vs 2.7 | 5.7 vs 5.4 | 6.9 vs 6.4 |
| combined | 7 vs 6.5 | 9.5 vs 9.1 | 11.4 vs 10.9 |
| folder | 2.7 vs 2.7 | 4.3 vs 4.3 | 5.3 vs 5.2 |

Every other unscoped query moved by 0.3 to 0.5 ms likewise; folder is scoped and reads its own sources row.
