---
cairn: log
change: as-fast-as-notmuch
landed: 2026-11-21
---

# Flag counts in one pass

The `is:unread` count of cairn/changes/as-fast-as-notmuch, 117 ms at 517k against notmuch's 13 (log 2026-11-21-system-flags-from-the-store): `unread` answers placements and a hit is unread only when none of its placements holds `\Seen` (SEARCH §8.2), so each of its 149,041 rows sought the hit's other placements, about 785 ns a row.

- **One pass** (src/client/search.rs `count_flagwise`, src/client/search/compile.rs `flagwise`, `scope_marks`): a query of system flags alone beside the terms its scope implies, over a scope of kinds and accounts, reads the scope's live placements once off `items_by_sort`, which holds `items.marks`, marking every hit in a bitmap of public ids per account and, per bit the query tests, the hits any of whose placements holds it; the query's `and`, `or` and `not` then combine the bitmaps, `not` taking a set away from every hit. A `collection:` or `folder:` scope keeps the plan: a hit's other placements may lie outside it.
- **The choice** (src/search/plan.rs `PimdirPlan::scans_flags`, I/O-free): the pass, 104 ns a placement (`PimdirPlanCosts::flags_row`), against the cheapest set a driver could read, 785 ns a row of `unread` (`unread_row`) and 330 of `marked` (`marked_row`), a capped estimate counting the whole store, and the pass when no set could drive (`not is:unread`, which the plan read over every item). A first version passed for every query reading `marked` and made `is:flagged` slower (41 to 57 ms): `marked`'s set is exact, its 20,399 rows cheaper than the pass.
- The bitmap helpers (`hit`, `unite`, `intersect`, `subtract`, `hits_counted`) are shared with the text count's.

## Proof

- src/search/plan.rs `a_flag_count_passes_over_the_store_unless_its_set_is_cheaper` pins both sides (an unread set of 2,000 rows in 520k keeps its set, 149,041 passes, a small `marked` set keeps it, a capped one or none passes) and fails with a pass chosen always.
- tests/search_plan.rs now checks that the unread count over its store, two thirds unread, passes over `items_by_sort` alone; it checked a read of `items_unread` before.
- A bit tested the wrong way (`!=`) or a `not` read as an `or` fails `every_refresh_vector_answers`, whose lagging flag steps count unforced.

## Measured

f6 store, core 28 (core 12 held by another import), hyperfine means of 15 after 3 warm-ups, ms, 62e8ab5 then this tree, every count equal: `is:unread` 126 / 69, `kind:mail is:unread` 201 / 54, `is:read` 167 / 60, `not is:unread` 462 / 57, `is:flagged` 49 / 58 then 43.1 / 44.4 in a 20-run A/B (the first gap was the machine), `is:unread is:flagged` 68 / 69, `is:answered` 53 / 54. notmuch: `tag:unread` 13.

## Left

The pass costs 104 ns a placement where `count_placed`'s, reading no marks, costs 77: a per-hit mark in the store, or `items_by_seq_global` covering `marks`, would answer "every placement lacks `\Seen`" by a seek per unread row and bring `is:unread` near its 4 ms walk of `items_unread`, a canonical schema change. `is:unread california` (80 ms, notmuch 4.9) mixes a flag and a text term and takes the plan.

Capabilities moved: search.md (a new requirement, "A count of system flags alone passes over its scope once when that is cheaper").
