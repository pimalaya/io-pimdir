# Round 3 (2026-11-22): addresses, the open floor, snippets

Goal from the owner, unchanged: beat notmuch on every search measure. Bench as in round-2.md (1M store runs/x2c, core 12, hyperfine medians of 5; compare script recreated as cmp.sh in the session scratchpad: md5 of count and first page against base, then count / page / threads medians).

## Built (uncommitted until noted)

- `@` address terms off the hit columns: index.hits (magic PIMHITS3, header word 13) holds per store item id its index placement, two to a word; `Column::Address` reads `store.item_address` by its address index and maps each item to its record, no join to items or collections. A domain (`@example.org`) still plans. 1M: from:jeff.dasovich@enron.com 23.6 / 29 / 29.8 to 7.2 / 8.4 / 10.2 ms (notmuch 13.2 / 18.1 / 19.8); combined 7.4 / 11 / 11.9 (was 8.2 / 12 / 12.7).
- The store attached in SQLite's shared cache (reader opens with `SQLITE_OPEN_SHARED_CACHE`, the index's ATTACH with `cache=shared`): the store schema is parsed once per process, the ATTACH going from 0.75 ms to 0.03 ms.
- The canonical schemas as built constants (`sql::DECLARED`, `sql::search::DECLARED`): build.rs compiles src/client/schema/declared.rs (the migration parser, moved out of schema.rs, `#[cfg(test)]` in the crate) and writes each object's normalised text; the check compares SQLite's recorded text word by word against it (`schema::same`), allocating nothing. Reader open 2.0 to 1.4 ms.
- Snippets: `fold_chars` folds ASCII text in one pass (it allocated a String per character): a 50-hit page of kiwi spent 7 ms in hits, now 5.6.
- Probe sets for plain terms: a page off the columns reads a plain term's FTS5 rowids into bitmaps when it matches at most 1,024 bodies and as many summaries (`PROBE_SET_MAX`, `LIMIT` 1,025), where `matched` probed each hit (~25 µs). Rare word page 9.3 to 5.6 ms.

## Where a query's time goes (kiwi count, fresh process, µs)

reader open ~1,400 (SQLite parsing pimdir.db's schema in `prepare_connection` ~950, the schema check ~380), index connect ~390, `held` ~150, count ~380, coverage ~700 of which `ROLL_COVERAGE_ALL` ~570 (a full scan of 6,622 `sources` rows, 121 pages: intrinsic to SQLite, 0.6 ms in its own CLI too; mmap gave nothing). A covering index on `sources(covered_at, covered_since, covered_until)` would need a canonical store schema change (pimdir repo, owner).

A page adds per hit: placements ~10, `matched` ~25 (an FTS5 probe per hit), snippet 30 to 60 after the fold fix (blob read ~15, MIME extraction ~5 to 20).

## Tried and dropped

- `PRAGMA mmap_size` on the query connection: no gain on any stage.

## Table after the first commit of the round

1M store x2c, core 12, hyperfine medians of 5, ms, count / page / threads; notmuch 0.40 re-measured the same hour (runs/x2/nm); every count and page md5 equal to base's.

| query | notmuch | base b080e14 | new |
| --- | --- | --- | --- |
| all mail | 54.4 / 263.4 / 278.9 | 76.5 / 16.1 / 17.1 | 6 / 12.8 / 14.3 |
| unread | 21.2 / 79.7 / 85.8 | 105.5 / 19.4 / 20.9 | 8.1 / 11.5 / 13.6 |
| flagged | 6.3 / 16.3 / 18.6 | 78.8 / 44.8 / 52.7 | 6 / 8.3 / 10.3 |
| from address | 13.3 / 17.3 / 19.1 | 23.6 / 27.8 / 29.3 | 5.9 / 7.5 / 8.9 |
| from name | 3.7 / 7.9 / 9.7 | 12.4 / 85.3 / 87.7 | 5.8 / 9.2 / 10.1 |
| subject word | 5.8 / 14.8 / 16.7 | 25.3 / 34.7 / 45.9 | 6.6 / 9.9 / 12.2 |
| common word | 87.6 / 287.3 / 302.2 | 205.4 / 30.5 / 37.5 | 52.5 / 61.4 / 62.1 |
| mid word | 9.3 / 26.9 / 29.2 | 34.1 / 44.5 / 51.1 | 8.2 / 10.6 / 12.5 |
| rare word | 2.9 / 4.4 / 6 | 4.5 / 9.3 / 10.2 | 3.6 / 5.6 / 7.3 |
| phrase | 17.1 / 27.5 / 29.5 | 30 / 37.6 / 44.1 | 9.9 / 14.2 / 15.9 |
| two words | 8 / 17 / 18.5 | 22.7 / 118.2 / 115 | 7.9 / 11.4 / 12.5 |
| negation | 10.1 / 22.9 / 25.5 | 33.3 / 58.5 / 70.7 | 8.6 / 12.9 / 13.8 |
| prefix | 5.4 / 12.8 / 13.9 | 18.4 / 104.1 / 103.2 | 5.7 / 9.4 / 11.5 |
| date month | 24.9 / 38.9 / 42.1 | 74.5 / 92.3 / 93 | 7.6 / 10.1 / 12.6 |
| combined | 4.8 / 6.7 / 7.7 | 80.2 / 87 / 88.4 | 7.5 / 9.6 / 11.5 |
| unread word | 7.1 / 14.2 / 16.7 | 115.7 / 61.3 / 74.8 | 8 / 11.5 / 13.9 |
| folder | 2.3 / 3.4 / 4.1 | 3.5 / 5.9 / 6.8 | 2.9 / 4.2 / 5.6 |

Still behind: from name, rare word, combined and folder on all three; subject word, prefix and unread word counts by under 1 ms. The page measure is not like for like: notmuch prints message ids alone (`--output=messages`), pimdir each hit's title, placements and a snippet cut from its body.
