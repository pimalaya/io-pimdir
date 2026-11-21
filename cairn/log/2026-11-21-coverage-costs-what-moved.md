---
cairn: log
change: as-fast-as-notmuch
landed: 2026-11-21
---

# Coverage costs what moved

Phase 2's coverage half of as-fast-as-notmuch (pimdir performance-audit-2026-11-05 F8). On the Enron bench (517k messages, 3,311 collections) a coverage cost 69 ms per answer: `list_coverage` once per collection (20 ms), `COUNT_UNINDEXED` planned over `items_by_seq` per collection (41 ms), the ids bound per statement (3 ms).

- The scope's collections are read once (`SCOPE_COLLECTIONS`) and bound once; the canonical `coverage` and `cut_before` run as their text stands. A draft of this change ran them with the slot rewritten to every collection for a whole-store query, saving the bind; it was dropped so the crate keeps running canonical text as signed (search.md, the statements requirement).
- The temporal axis is one statement of the crate's own, `ROLL_COVERAGE`, equal to `PimdirSearchTemporal::over` source by source (src/search/coverage.rs grid test, up to three sources over every range); `search_temporal` and its loop are gone, `PimdirReader::search_coverage` reads the same row.
- `COUNT_UNINDEXED` is forced onto `items_by_changed` and asked only while `behind` is above 0 (src/sql/search.rs `the_unindexed_count_seeks_the_change_stamp`, failing on the previous text).
- `PimdirIndex` caches up to 64 scopes keyed on both databases' `PRAGMA data_version`, emptied by `refresh` (`a_coverage_is_read_again_once_the_store_or_the_index_moved`). The CLI answers one query per process and never reuses it.

Left: the canonical `coverage` stays one `items_bodiless` seek per collection of the scope; a per-collection stale set refreshing the cache (planner-design.md §4) is not built, the cache dropping everything on any commit. The bench's query table measures the gain once the planner lands.

Capabilities moved: search.md (the coverage requirement). Checks: the full suite, 1,039 passed.
