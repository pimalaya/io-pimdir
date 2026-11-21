---
cairn: log
change: as-fast-as-notmuch
landed: 2026-11-21
---

# A query costs what it answers

Phase 2's planner of as-fast-as-notmuch (pimdir performance-audit-2026-11-05 F1), from cairn/changes/as-fast-as-notmuch/planner-design.md. Every compiled query opened with a `live` CTE over every live item, probed each against every term, read every hit before cutting a page and grouped every hit for a threaded page: 125 ms at 517k Enron messages whatever the answer, 13.8 s for page 1 of all mail threaded.

- src/search/plan.rs, I/O-free: the scope from the conjuncts, estimates capped at 100,001, the `Set`, `Walk` (with a budget and a fallback set) and `Scope` drivers, `PimdirWalk` merging the scope's collections in key order; `PimdirSearchSort` and `PimdirSearchCursor` moved there, re-exported at their old paths, the cursor token unchanged.
- src/client/search/compile.rs renders the plan; `live`, `with` and `select` are gone. src/client/search.rs runs `search`, `count`, threads and the tag path on it.
- tests/search.rs runs every query vector under the natural plan and each forced driver (`PimdirIndex::force_driver`, hidden); breaking the walk's pass column fails 216 walk cases. tests/search_plan.rs: 3,000 messages, two accounts, items filed twice, every driver paging alike in both dated orders, and plan checks (no scan of the store's items but a scope count, every step seeking `items_by_sort`, page 1 of `kind:mail` in one to six steps, `kiwi` in none).

Found in review, fixed before landing: the plan's scope intersected every `collection:` conjunct, so `collection:home/INBOX collection:home/Kept` answered nothing where item 12, filed in both, answered before (and notmuch answers `folder:a folder:b` alike). The first such conjunct alone narrows the scope; a later one is probed. pimdir gains the two cases (both orders, `folder:` once) and SEARCH §8.2 states a hit's key, read among its placements in the query's collections where one item's placements differ, which STORAGE §9.1 keeps from happening for mail; the vectors reach this crate with the next re-vendoring, and the fix was checked against them in a scratch copy (failing without it).

Left: counts over broad queries (`kind:mail`, `is:unread`, `enron`) still pass over the scope, `relevance` still reads every hit, the cost constants are the design's, unmeasured. The bench's query table measures the rest.

Capabilities moved: search.md (a new requirement, the page and shallow-statement requirements restated).
