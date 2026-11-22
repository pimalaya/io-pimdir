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

## Second step

- The store schema check walks `sqlite_schema` in name order beside `sql::DECLARED`, borrowing each text, no map built (folder count ~0.1 ms). tests/stale_store.rs `a_missing_object_is_named_before_an_extra_one` keeps the error naming order.
- Snippets read `PimdirExtract::text`: a message's body and note without its address fields and attachments (the attachments walk parsed the message a second time). tests/search_pure.rs `a_snippet_reads_the_body_the_index_reads` compares the two over every mail fixture. Per hit of a folder page, snippet 28 µs (blob 6.7, extraction 11.5, chunks and fold ~10) and placements 9 before.
- Bench note: the machine drifts (powersave governor): notmuch's folder count read 2.3 then 3.9 ms an hour apart. Head-to-head numbers now come from one hyperfine run holding both commands per measure (vs.sh in the session scratchpad: notmuch then pimdir, medians of 10).

Head-to-head after the first step (pimdir-new8, ms, notmuch vs pimdir), count | page | threads:

| query | count | page | threads |
| --- | --- | --- | --- |
| all mail | 54.3 vs 5.7 | 263 vs 12.3 | 279.6 vs 13.9 |
| unread | 21.4 vs 7.6 | 79.9 vs 10.8 | 86.8 vs 12.9 |
| flagged | 5.7 vs 5.4 | 16.9 vs 8.7 | 18.3 vs 10 |
| from address | 12.6 vs 5.8 | 17.8 vs 8 | 19.1 vs 8.9 |
| from name | 4.1 vs 5.9 | 7.9 vs 8.6 | 9.9 vs 10.4 |
| subject word | 5.4 vs 6.3 | 15 vs 10.3 | 16.5 vs 12 |
| common word | 88 vs 51.4 | 287.2 vs 60.8 | 302.7 vs 62.2 |
| mid word | 9.1 vs 6.8 | 27.6 vs 11.3 | 29.4 vs 12.6 |
| rare word | 2.4 vs 3.1 | 4.2 vs 5.6 | 6.1 vs 7.5 |
| phrase | 16.9 vs 9.5 | 27.8 vs 13.8 | 29.9 vs 16.2 |
| two words | 8.1 vs 7.6 | 17.3 vs 11.6 | 18.6 vs 12.6 |
| negation | 9.3 vs 8 | 23.5 vs 12.1 | 25.8 vs 13.8 |
| prefix | 5.8 vs 5.5 | 12.2 vs 9.3 | 14.4 vs 11.5 |
| date month | 23.9 vs 7 | 39.6 vs 10 | 42.3 vs 12 |
| combined | 5.1 vs 6.8 | 6.4 vs 9.5 | 8.3 vs 11.5 |
| unread word | 6.7 vs 7.2 | 14.8 vs 12 | 16.6 vs 13.8 |
| folder | 2.3 vs 2.7 | 2.9 vs 4.2 | 4 vs 5.5 |

The `from` column holds the names the body's From header parses to (src/search/extract/mail.rs), not the store's `item_address` rows, so a `from:` name term cannot be answered off the store exactly; FTS5's column filter over `dasovich`'s 63,150 bodies costs 2.8 ms of the from name count (notmuch reads a 22k posting list of its own `from` prefix).

Head-to-head after the second step (pimdir 04d3211, ms, notmuch vs pimdir), count | page | threads:

| query | count | page | threads |
| --- | --- | --- | --- |
| all mail | 54.5 vs 5.9 | 263.9 vs 12.7 | 280 vs 14 |
| unread | 21.6 vs 7.9 | 80.2 vs 11 | 86.3 vs 13.1 |
| flagged | 6.4 vs 6 | 16.4 vs 8 | 18.9 vs 10.1 |
| from address | 13.1 vs 6.5 | 17 vs 7.5 | 19.6 vs 9.2 |
| from name | 4.1 vs 6 | 7.9 vs 8.5 | 10 vs 10.1 |
| subject word | 5.3 vs 6.4 | 15.1 vs 10.1 | 17.1 vs 11.8 |
| common word | 89.2 vs 52.7 | 288.4 vs 60.1 | 304.2 vs 63.4 |
| mid word | 8.8 vs 7 | 27.9 vs 11.1 | 29.6 vs 12.4 |
| rare word | 2.8 vs 3.5 | 4.2 vs 5.4 | 6 vs 7.3 |
| phrase | 16.5 vs 10.5 | 28.6 vs 14.3 | 29.5 vs 15.6 |
| two words | 7.7 vs 7.3 | 17.3 vs 11.6 | 19 vs 12.8 |
| negation | 9.3 vs 8.2 | 23.4 vs 11.4 | 25.2 vs 13 |
| prefix | 5.3 vs 5.2 | 12.7 vs 9.7 | 14.1 vs 10.9 |
| date month | 24.4 vs 7.7 | 40 vs 10.2 | 42.5 vs 12.1 |
| combined | 4.7 vs 6.6 | 6.4 vs 9.5 | 7.7 vs 10.7 |
| unread word | 7.2 vs 7.8 | 14.9 vs 12 | 16.8 vs 13.8 |
| folder | 2.3 vs 2.7 | 3.4 vs 4.1 | 4.1 vs 5.2 |

Ahead on all three for 11 of 17 (subject word and unread word on page and threads). Behind: from name (FTS5 column filter, above), combined (the same from: term), rare word and folder (the process floor: SQLite parsing pimdir.db's schema ~0.95 ms and index.db's ~0.3, coverage's sources scan ~0.57 ms for an unscoped query; a page's per-hit presentation, placements ~9 µs and snippet ~20 µs a hit), subject and unread word counts by 0.6 to 1.1 ms.

## Left, each needing the owner

- A covering index on `sources(covered_at, covered_since, covered_until)` in the canonical store schema (pimdir STORAGE): an unscoped coverage reads ~40 index pages instead of 121 table pages (~0.4 ms off every unscoped count and page).
- A posting list of the `from` column's tokens: the index keeps no copy of the text (contentless FTS5), so index.hits cannot derive it at its rebuild; it needs the refresh to keep it incrementally (an io-pimdir-owned table beside the canonical schema, or a canonical one). Would take from name and combined to about the floor.
- The page measure is not like for like: notmuch `--output=messages` prints ids alone, pimdir each hit's title, placements, flags and snippet. A presentation without snippets (an option, or a JSON field set) would put rare word and folder pages under notmuch's, but changes the CLI contract.

## Owner decisions (2026-11-22, round 4 start)

- Covering index on `sources(covered_at, covered_since, covered_until)`: yes, in the canonical store schema.
- `from` token posting list kept by the refresh: yes, io-pimdir-owned (beside the canonical index schema, not in pimdir's spec).
- Snippet-less presentation: no, the CLI contract stays; only the per-hit cost gets cheaper.
