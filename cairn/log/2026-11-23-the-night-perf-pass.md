---
cairn: log
change: as-fast-as-notmuch
landed: 2026-11-23
---

# The night perf pass

After a-smaller-pimdir (pimdir cairn/changes/a-smaller-pimdir), five independent agents each took one lead on a scratch copy of db31aab and the 517k Enron store; the lead re-measured every patch head to head at 1M on core 12 (notmuch, the binary before, the binary after, one hyperfine run), kept it only beyond noise with the 51 outputs of the 17 bench queries identical byte for byte, and ran the whole suite. Two of the five carry their own entry (2026-11-23-the-hit-columns-patched, 2026-11-23-a-build-merges-once); this one records the other three.

- **Pages and threads** (cde1896): `search` and `threads` in one read transaction, where each statement took and dropped both databases' read locks; a page's threads read in two statements (`HIT_THREADS`, `THREADS_HELD`) where each hit and each thread took three; chunks cut lazily up to the first naming a word, the CR/LF scan a word at a time (`chunk::Chunks`, test `a_line_end_read_a_word_at_a_time_is_the_first_cr_or_lf`); the text output written once. 1M under load: threads 0.7 to 1.8 ms faster, pages 0.3 to 1.2.
- **Proven body hits** (187f31f, 98c2eca): a hit the hit columns read a positive text term off one of its bodies for is `body` without a probe, so a page whose hits are all proven reads no probe set; the calendar check of a dated query reads the mapped kinds; the joint FTS5 AND steps column-filtered terms last. Test `a_hit_matches_alike_off_the_hit_columns`, which fails when a summary match is marked so. 1M: text pages and threads 1 to 2.6 ms faster, combined page 12.26 to 9.61.
- **The query floor** (f8eae7a, 277a31b): SQLite's MEMSTATUS off, a query-only index left to the process exit, `temp.param` created at a read connection's first bind, a two-verb clap root (`the_query_root_reads_as_the_whole_tree`), store and index judged by their meta row when their schema is canonical, `COVERAGE_HELD`, a one-statement hit stamp. 1M: 0.3 to 1.4 ms off nearly every measure.

Measured and left: `mmap_size` on query connections, larger lookaside, MULTITHREAD, mallopt, `INDEXED BY` in `THREADS_HELD`, the posting list cut into rowid ranges for the combined count, a bare word added to the joint AND (each within noise or slower). Static linking saved 0.15 to 0.21 ms a query (the dynamic loader is 7.8% of a count) and is a packaging decision for the owner. What is left of a count's floor is SQLite parsing the two schemas (44% and 9.5% of its user cycles).

Capabilities moved: search.md (the threads requirement, what a hit matched, the hit columns' requirement). Checks: the three tests named above; the whole suite, 1,171 passed after the five patches.
