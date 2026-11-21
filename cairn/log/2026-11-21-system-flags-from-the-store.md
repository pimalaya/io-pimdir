---
cairn: log
change: system-flags-from-the-store
landed: 2026-11-21
---

# System flags from the store

The crate follows pimdir e2b7f49 (cairn/log/2026-11-21-system-flags-from-the-store.md there), the owner's decision of 2026-11-21 recorded in cairn/changes/as-fast-as-notmuch/proposal.md ("Decided") on finding F6 of the performance audit: `is:read`, `is:unread`, `is:flagged`, `is:answered`, `is:draft`, and a `tag:` or `flag:` naming one of the five system flags, read the store's `items.marks` as it stands, keywords the index. search-draft-16 is edited in place, no number moves. spec/ is re-vendored at e2b7f49 (the `flag` table's `CHECK`, `unread.sql` and `marked.sql` added, `flagged.sql` and `insert_flag.sql` commented, queries/signatures.json, refresh.json at `v` 4 with its schema, vectors/README.md and the manifest) and spec/REVISION names it.

- **The query** (src/client/search/compile.rs): a system flag's set is the canonical `marked` (its bit) or, for `is:unread`, `unread`, embedded and lifted to `(account, seq)`; its estimate the same statement counted up to 100,001; its condition over a hit `marks & bit = bit` on the hit's live placements in its account. A keyword reads `flag` as before. `PimdirQueryPredicate::Flag` stays the one predicate and `is:unread` a negated `\Seen`.
- **The plan** (src/search/plan.rs): `PimdirPlanEstimate::Marked` and `Unread` beside `Flag`, now a keyword's; a conjunct `not` over a `\Seen` leaf, or such an operand of a conjunct `or`, is a set like a positive term, so `is:unread` drives from `items_unread` where it passed over every live item. `unread` answers placements, and a hit is unread only when none of its placements holds `\Seen` (SEARCH §8.2), so its set is no exact one: `PimdirPlan::are_exact` answers false and `Compiled::skip` keeps every term of such a set probed per hit, as `probe_row` costs it.
- **The count** (src/client/search/compile.rs `count_set`): a set of system flags over an unbounded scope seeks no live placement per hit, each of its rows being one.
- **The refresh** (src/search/refresh.rs): `ReplaceFlags` carries a placement's keywords alone, the schema refusing the five.
- **The vectors** (tests/search.rs): refresh.json read at `v` 4, a `lagging` step's queries answered before its refresh; every query stating `hits` asked under the plan's driver and each forced one, listed and counted.
- **tests/search_plan.rs**: a walk of `items_unread` is no scan of the items (it holds the unread rows alone, the set itself); the scope count's covering read of `items_by_sort` is now checked on `-is:flagged`, `is:unread` counting off `items_unread`.

## Proof

- Against 8276b87's code, refresh.json's new case fails: the refresh inserts `\Seen` into `flag`, which the `CHECK` refuses, and the lagging `tag:\Flagged` reads an index one write behind.
- With `PimdirPlan::are_exact` answering true for an `unread` set (its term skipped as a driver's), tests/search.rs fails "every kind of write an owner makes" step 2 (`is:unread kind:mail`) and this case's step 2, every queries.json case of `is:unread`, and `a_message_held_without_its_body_counts_only_against_what_reads_it`.

## Measured

On the Enron tier x1 (518,401 messages in 3,312 folders), core 12 idle, a fresh import by this tree's pimbench (sync 527 s wall, index 180 s, pimdir.db 889 MB, index.db 1,878 MB) kept at /home/soywod/code/pimalaya/.bench-notmuch/f6/store; against 8276b87 built beside it and indexing a copy of the same pimdir.db (index 196 s, index.db 1,931 MB). Hyperfine medians of 10 after 2 warm-ups, process start included, ms, count / page of 50 / threaded page of 50, 8276b87's in brackets, notmuch's of the earlier run after them:

- `is:unread` (149,041): 116 / 12.7 / 15.2 (572 / 9.6 / 13.2; notmuch 13 / 43 / 44)
- `is:flagged` (20,399): 44.5 / 30.6 / 34.1 (28.3 / 14.6 / 18.8; notmuch 4.8 / 10 / 11)
- `is:unread california` (11,479): 81.4 / 40.8 / 51.7 (92.6 / 43.2 / 54.3; notmuch 4.9 / 9.2 / 10)

One run each, no A/A check. The unread count measured first at 185 ms; a count driven by a system flag's set over an unbounded scope now seeks no live placement per hit (`Compiled::count_set`), the set's rows being live placements already. Its remaining cost, by sqlite3 on the same store: the `items_unread` walk 4 ms, the hits made distinct and joined to their collections 45 ms, the per-hit probe of the other placements (`NOT EXISTS ... marks & 1 = 1`, a seek on `items_by_seq_global` and a table row per placement) about 70. `is:flagged` doubles: `marked` walks every live entry of `items_by_sort` for the estimate and again for the set, where the index's `flag` table was a seek (the spec's "no new store index" choice); the unread page pays the `items_unread` estimate, 3 ms.

## Found in the spec

- `unread`'s plan reads `SCAN i USING INDEX items_unread`, not `COVERING`, though it walks the 149,041 unread rows in 4 ms on the bench: the plan line overstates the cost.
- `is:flagged`, `is:answered` and `is:draft` cost more than they did from the index (above): the partial `items_flagged` the spec left out is what would make `is:flagged` a seek again.
- The per-hit rule makes every `unread` hit a probe of its other placements, about half the unread count's cost; a store answering "the item's placements in an account all lack `\Seen`" by one seek (a per-seq mark, or `items_by_seq_global` covering `marks`) would remove it.

Capabilities moved: search.md (a new requirement, "System flags are read from the store", the vectors' and the planner's requirements, `ReplaceFlags`), spec-fidelity.md (the revision).
