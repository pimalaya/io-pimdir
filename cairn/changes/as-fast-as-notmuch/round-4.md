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

## The from posting list (f38858c)

`pimdir_from`: a contentless FTS5 table of `object_text`'s `from` column alone, `detail = none`, same tokenizer, same rowid, written and dropped with the body's `object_text` row, created by the build, optimized by `erase`, left out by the shape check like any other table. Off the hit columns (count bitmaps, page probe sets), `{from} : "w"` and `{from} : "w" *` with `w` ASCII alphanumeric read it; a phrase errs under `detail = none` (checked in SQLite's shell), so phrases, other words, summaries and ranked reads stay on `object_text`. A missing-rowid delete on a contentless_delete table is harmless (shell). An index without the list is `PimdirIndexHeld::Unposted`: the indexer rebuilds, a query client answers off `object_text`.

## Tried and dropped

- blake3 `std` (runtime detection): the crate's build already detects without it; 1 KB 1.10 µs, 3 KB 2.18, 32 KB 6.27 per hash with and without.

## The rebuild kept its hit columns (fixed)

Rebuilding x2d for the posting list left index.hits at its old mtime: the new index.db took the old inode, and an unchanged store gives the rebuild the same change, purge and id counts, so the stamp accepted the old cache. Harmless here (the rebuild drew the same ids), wrong in general. The build removes index.hits with index.db, and the stamp's file word is the inode rotated and XORed with the birth time (std `created`, statx on Linux). Guard: tests/search_plan.rs `a_rebuild_drops_the_hit_columns`, failing without the removal.

## Head to head after the posting list (new11, x2d rebuilt in 10 min 35 s, 157 s user; index.db 3,776 MB against 3,758)

| query | count | page | threads |
| --- | --- | --- | --- |
| all mail | 53.3 vs 5.7 | 261.9 vs 11.5 | 277.6 vs 13.6 |
| unread | 20.7 vs 6.7 | 79.9 vs 10.5 | 84.8 vs 11.7 |
| flagged | 6.2 vs 5.5 | 16.5 vs 7.5 | 18.9 vs 9.3 |
| from address | 12.5 vs 5.5 | 17.4 vs 7.4 | 19 vs 8.3 |
| from name | 3.7 vs 3.6 | 7.9 vs 6.5 | 9.9 vs 8.2 |
| subject word | 5.7 vs 6.2 | 14.4 vs 9 | 17 vs 11.1 |
| common word | 88.1 vs 52.2 | 287.2 vs 60.5 | 302.2 vs 61.8 |
| mid word | 9.3 vs 6.9 | 26.6 vs 9.9 | 30.3 vs 12.1 |
| rare word | 2.3 vs 2.7 | 4.5 vs 5.3 | 6 vs 6.9 |
| phrase | 16.7 vs 8.8 | 27.9 vs 13.7 | 29.2 vs 14.7 |
| two words | 7.9 vs 7.1 | 16.7 vs 10.2 | 19.3 vs 12.2 |
| negation | 9.3 vs 7.7 | 23.8 vs 11.6 | 25.2 vs 12.5 |
| prefix | 5.7 vs 5.1 | 12.4 vs 8.8 | 14 vs 10.2 |
| date month | 24.6 vs 7 | 39.5 vs 9.9 | 42.2 vs 11.3 |
| combined | 4.7 vs 6 | 6.8 vs 9.4 | 7.7 vs 10.3 |
| unread word | 7.2 vs 7.4 | 14.7 vs 11.1 | 16 vs 12.8 |
| folder | 2.5 vs 3 | 3 vs 3.8 | 4.5 vs 5.5 |

From name leads on all three now (12 of 17). Trailing: combined, rare word, folder on all three; subject word and unread word counts.

Tried and dropped: taking a posted `from:` word out of the joint FTS5 `AND` (combined: count 8.1 to 10.6 in one run): alone, `california` is decoded whole, where the joint `AND` seeks through its doclist.
