---
cairn: log
change: as-fast-as-notmuch
landed: 2026-11-21
---

# The planner measured

Phase 2's tuning of as-fast-as-notmuch, on a copy of the 517k Enron store (io-pimdir 5dbfe23 as the reference, core 12, hyperfine medians). The planner's first constants were the design's guesses; a full-text probe cost 30 µs against the 6 assumed, so a walk probing two text terms ran past its budget and fell back to probing one term against every hit of the other: two words paged in 1,424 ms and a negation in 1,277, slower than before the planner.

- Text terms probed per candidate read their sources first (`temp.term_source`, from the doclists alone), each probe a seek of about 4 µs.
- No `bm25` outside `relevance` (1.8 µs a match); what a hit matched read from the same ids.
- The walk: a key-ordered queue of collections, first steps of 8 rows doubling; expected rows weighed by the store's extent.
- Measured costs in `PimdirPlanCosts`; a date term always a set; `CROSS JOIN` on set pages and the date set (SQLite had chosen to scan `items_by_sort`).
- Counts implied by the scope alone through a per-account bitmap of public ids: all mail 192 to 40 ms.
- Coverage over every collection through pimdir's new canonical `coverage_all` (pimdir 0158220) and `ROLL_COVERAGE_ALL`: 9.05 to 0.44 ms. The tuning first ran the canonical text with its slot removed as the crate's own; replaced by the canonical statement so the crate states no own text.
- The schema check reads the canonical objects from the migrations' text: 3.06 to 0.60 ms an open.

Query table after, ms (count / page / threads), notmuch first: all mail 30 / 147 / 143 against 40 / 15 / 19; rare word 2.4 / 3.8 / 4.8 against 3.8 / 14 / 17; folder 2.4 / 3.2 / 4.5 against 3.3 / 9 / 12; two words 6 / 11 / 12 against 226 / 91 / 107; unread 14 / 48 / 50 against 583 / 16 / 22. The rest in cairn/changes/as-fast-as-notmuch (untracked) and the tuning's raw files under .bench-notmuch/tune/out.

Still short: text counts (the placement join by hash; owner decision 2026-11-21, integer object ids), the unread count (owner decision, system flags from the store's marks), pages of text terms (walks of 4k to 10k rows), about 7 ms of snippets per page. Not kept: memory-mapped databases, a 64 MiB page cache.

Capabilities moved: search.md (a new requirement). Checks: the full suite, every vector under every forced driver, the walk's proptests, tests/search_plan.rs's plan checks, the schema parse test.
