# Round 2 (2026-11-21 night): the hit columns

Goal from the owner: beat notmuch on every search measure, free hands, unsafe allowed.

## What is built (uncommitted in the io-pimdir tree)

- src/client/search/hits.rs: `index.hits`, a cache beside index.db written by `refresh` whenever the index holds the store as it stands and the cache's stamp differs (store_change, store_purges, max placement id, max object id). Flat u64 words, mmapped (rustix `mm` feature added), never truncated in place (written beside, renamed over).
- Columns: per index placement id its date (sort key `YYYY-MM-DDTHH:MM:SSZ` read as the number YYYYMMDDHHMMSS, exact flag in the header), seq, account, marks, kind; per object id its placement (or the multi list); per hit (account, seq) the greatest date, -1 when undated.
- `columnar()`: a count of system flags, text terms (FTS5 rowids only, no placement join) and dates over mail, under a scope of kinds and accounts, combined as bitmaps by the existing `flagged()`. Hooked first in `counted()`.
- `columnar_page()`: a first dated page: the hits' keys, `select_nth` for the threshold, the candidates written to `temp.hit_page`, the existing `met_rows` sorts and cuts exactly. Hooked first in `ordered()`.
- Proof: the whole suite passes; a probe panic shows 104 query vectors take the path.

## Bench

1M store rebuilt at b080e14 as .bench-notmuch/runs/x2c (old runs/x2 refused, schema moved). Results below when measured.

## Second build (lazy every, joint `and` of text when `split` is empty, a first page before any plan)

1M store x2c, core 12, hyperfine medians of 5, ms, count / page / threads; base is b080e14, every count and page identical to base's.

| query | notmuch | base | new |
| --- | --- | --- | --- |
| all mail | 57.2 / 266 / 278.2 | 74.2 / 16.1 / 16.9 | 6.6 / 15.7 / 17 |
| unread | 20.6 / 80 / 85.1 | 99.5 / 18.1 / 20.2 | 7.7 / 13.1 / 19.3 |
| flagged | 6.7 / 17.5 / 19.6 | 76 / 46.9 / 52 | 6.4 / 9.7 / 45.4 |
| from address | 13.2 / 18.1 / 19.8 | 25.7 / 28.6 / 29.9 | 24.2 / 28.8 / 30 |
| from name | 4 / 8.2 / 9.8 | 14.7 / 91 / 90.4 | 7.2 / 11 / 16.1 |
| subject word | 5.3 / 15.1 / 16.3 | 27.2 / 35.9 / 46.4 | 7.5 / 12.1 / 20.8 |
| common word | 97.1 / 291.9 / 306.2 | 209.7 / 30.9 / 38.4 | 53.2 / 64.9 / 73.9 |
| mid word | 9.2 / 27.6 / 30 | 35.8 / 43.7 / 51.2 | 9.1 / 13.6 / 25.7 |
| rare word | 2.5 / 4.4 / 5.9 | 4.7 / 9.1 / 11.6 | 5 / 9.6 / 10.6 |
| phrase | 17 / 27.7 / 30 | 32.8 / 37.8 / 44.7 | 10.3 / 16.4 / 28.9 |
| two words | 7.6 / 16.9 / 19.3 | 23 / 118.7 / 117.1 | 9.4 / 13.5 / 32.7 |
| negation | 9.3 / 23.6 / 25.5 | 30.9 / 52.9 / 67.4 | 10 / 14.4 / 32.4 |
| prefix | 5.3 / 12.8 / 14.2 | 18.5 / 103.8 / 106.3 | 6.1 / 65.4 / 88.7 |
| date month | 23.8 / 39.2 / 41.7 | 77.8 / 94.2 / 96 | 8 / 12.1 / 22.8 |
| combined | 4.7 / 6.6 / 8.1 | 82.3 / 88.3 / 90.5 | 12.9 / 16.7 / 55.8 |
| unread word | 6.7 / 14.9 / 16.1 | 120.3 / 60.9 / 74.9 | 13 / 18.1 / 35 |
| folder | 2.5 / 3.3 / 4.3 | 3.7 / 6.8 / 7.9 | 4.6 / 5.6 / 7.8 |

notmuch's column is the 509fd83 run's (runs/x2, same machine). Left: threads still plan (estimates, terms read into temp tables), the prefix page (65), `@` address terms (not in the columns), the full record scan under flag and date terms (combined, unread word), the process floor (rare word, folder).

## Fifth build (threads off the columns, prefix probes as bitmaps, per-hit flags and date)

- `threads()` reads its batches off the columns (`columnar_batch`, now taking a cursor: the hits past the cursor's key, those on it counted in) and its members as the thread's seqs among the result bitmap (`columnar_members`), planning only if a batch falls back.
- On the early paths, a probed prefix term is read once into bitmaps of object ids and placement ids (`probe_sets`, `Sourced::Sets`); a probe per hit expanded the prefix again (prefix page 65 to 12 ms).
- The cache (magic PIMHITS2) holds per hit the union of its placements' marks and its mail date, with a header bit when every hit's dated mail placements share one date; an `and` whose other operands leave few hits (an eighth of the records) tests its flag and date terms on them instead of scanning.
- tests/search_plan.rs `every_driver_answers_alike_and_seeks` removes index.hits after its refresh: it watches the planned drivers, which the columns now answer before.

| query | notmuch | base | new |
| --- | --- | --- | --- |
| all mail | 57.2 / 266 / 278.2 | 79 / 15.9 / 17.6 | 6.7 / 14.7 / 16.4 |
| unread | 20.6 / 80 / 85.1 | 101.7 / 18.2 / 19.9 | 8.2 / 12 / 13.4 |
| flagged | 6.7 / 17.5 / 19.6 | 74.5 / 46.3 / 50.3 | 6.3 / 9.6 / 10.9 |
| from address | 13.2 / 18.1 / 19.8 | 24.1 / 28.1 / 30.4 | 23.8 / 28.4 / 29.9 |
| from name | 4 / 8.2 / 9.8 | 13.1 / 87.7 / 88.1 | 6.5 / 10 / 10.9 |
| subject word | 5.3 / 15.1 / 16.3 | 26.3 / 35.9 / 46.2 | 7.7 / 11.2 / 13.1 |
| common word | 97.1 / 291.9 / 306.2 | 210.2 / 30.1 / 37 | 54.8 / 63.9 / 66.8 |
| mid word | 9.2 / 27.6 / 30 | 35.1 / 44.7 / 53 | 7.7 / 12.2 / 14.5 |
| rare word | 2.5 / 4.4 / 5.9 | 4.7 / 9.4 / 10.8 | 3.9 / 9.6 / 11.1 |
| phrase | 17 / 27.7 / 30 | 29.8 / 37.8 / 44.8 | 10.5 / 16.1 / 16.9 |
| two words | 7.6 / 16.9 / 19.3 | 23.7 / 123.6 / 118 | 9.1 / 13.8 / 14.3 |
| negation | 9.3 / 23.6 / 25.5 | 30.1 / 53.4 / 66.7 | 9.2 / 13.3 / 15.8 |
| prefix | 5.3 / 12.8 / 14.2 | 18.7 / 103.9 / 104.5 | 6.2 / 11.6 / 13.3 |
| date month | 23.8 / 39.2 / 41.7 | 75.3 / 91.7 / 94.3 | 8.3 / 11.7 / 13.1 |
| combined | 4.7 / 6.6 / 8.1 | 87.7 / 92.6 / 91.3 | 8.2 / 12 / 12.7 |
| unread word | 6.7 / 14.9 / 16.1 | 117.9 / 61.2 / 76 | 8.4 / 14 / 15.4 |
| folder | 2.5 / 3.3 / 4.3 | 3.6 / 5.5 / 7.2 | 3.8 / 5.5 / 6.9 |

Still behind: the process floor (pimdir opens in about 3.8 ms where `notmuch --version` takes 3.2; perf cannot open events in this sandbox), `@` address terms (not in the columns: item ids are not mapped), and a page's presentation (snippets read each hit's body; notmuch's `--output=messages` prints ids alone). The common word page went 30 to 64 (still 4.5x notmuch's): its 500k hits are all candidates' keys read, where the walk met 50 quickly.

## Final state of the round (2026-11-21, 23:50)

The stamp also holds index.db's inode (header word 12), so a rebuilt index refuses the cache whatever ids it reaches. tests/search_plan.rs `the_hit_columns_answer_as_the_index_alone` compares counts, pages walked to their end in both dated orders and threads with and without index.hits (flag and date filters per hit included); the two logger tests hold a `SERIAL` lock. 1083 tests pass, clippy clean. Final 1M numbers match the fifth build's table within noise.

Tried and dropped: comparing the store schema's raw text before normalising (SQLite's recorded text never matched the migration slice, so every open normalised anyway, slower) and removing the per-word allocation of the migration parser (no measurable gain). The floor left is about 1 ms: SQLite parsing pimdir.db's schema on the reader and again on the index connection's attach, plus the schema checks.

Left, by gain: `@` address terms in the columns (map store item ids to placements; from address 23 against 13); the floor; the common word page (63 against base's 30, notmuch 292).

Not committed: the commit is staged in io-pimdir but gpg's pinentry timed out with nobody at the machine.
