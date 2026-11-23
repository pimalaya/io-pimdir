---
cairn: log
change: as-fast-as-notmuch
landed: 2026-11-23
---

# A build merges once

The index phase of the import (as-fast-as-notmuch, night perf pass of 2026-11-23). A 517k rebuild spent a quarter of its CPU in FTS5's incremental merges (`fts5IndexMergeLevel`, `fts5SegIterNext`, `fts5MultiIterNext` in a flat profile) and wrote 22.9 GB for a 1.78 GB index.db, a commit every 4,096 rows rewriting a page of every random-key index per row.

- **Runs share a transaction** until it folded 16,384 rows (`TRANSACTION_ROWS` in src/client/search.rs), the deadline checked after each run; a run stays `PimdirIndexRefreshOptions::batch` rows, so the memory does not move (batch 65,536 itself cost 12 s more CPU and 490 MB).
- **A build holds its merges back.** A pass from an empty index over a feed longer than a batch sets `automerge` 16 and `crisismerge` 64 on the three full-text tables (`MERGES_HELD`); a pass finding `automerge` 16 goes on with the build. The pass exhausting the feed merges each table into one segment by negative `merge` steps of 16,384 pages, one transaction each, the deadline checked between them, then sets FTS5's defaults back (`MERGES_SETTLED`). The index at rest is one segment a table where the old build left 14 and 7.
- 517k, three alternating runs each on one core: wall 121.8 to 92.2 s, CPU 90.6 to 75.6 s, write_bytes 22.9 to 16.5 GB, peak WAL 195 to 253 MB, index.db 1,775 to 1,862 MB (91 MB free pages). 1M (corpus x2, one run each, warm cache): wall 288 to 231 s, CPU 199 to 169 s, write_bytes 57.2 to 42.1 GB, peak WAL 253 to 554 MB, index.db 3,539 to 3,627 MB, one segment a table. Every index table and every full-text instance (189,884,778 of `object_text`) dumped identical but `index_meta`'s horizon; the 51 outputs of the 17 bench queries identical, from the base and the new binary.

Measured, not kept: `automerge` 0 (crisis merges alone), the same CPU at 517k but a whole level merged in one transaction at 1M (4.8 GB WAL, 5.1 GB index.db); a single `optimize` ending the build (7.9 GB written at 517k, a 1.65 GB WAL, no deadline); 65,536 rows a transaction (a quarter fewer bytes, a 1.2 GB WAL at 1M); a 64 MiB page cache during the build (sys 1 to 4 s less, no CPU, 67 MB more); `synchronous = NORMAL`, `mmap_size` 4 GB, FTS5 `hashsize` 16 and 64 MB (within noise).

Capabilities moved: search.md ("A refresh folds the feed in bounded, resumable batches"). Checks: tests/search.rs `a_build_merges_its_text_once_at_its_end` (fails on the old code: no merge setting held), the search, plan, erase, store_text, spec_drift, crash_reopen and walk_agrees suites.
