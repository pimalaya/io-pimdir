---
cairn: log
change: the-search-lines-close-for-the-freeze
landed: 2026-11-25
---

# The search lines close for the freeze

Follows pimdir's change of the same name (search-draft-18): the query language's last readings, the vectors pinning extraction, threads, `matched`, `complete` and pages, and the runners for them.

## What landed

- src/search/query.rs: `PimdirQueryDateRange::point`, whether a date value is one point, so a relative amount runs to `now` only as a single point and a side of a range spans its instant alone; `2w..2w` read as `2w` before, answering the two weeks before now where SEARCH §8.1 gives the empty span. `linked:` naming a role and no id after its colon (`linked:sender:`) is refused `invalid-value` over the value, where it read the link id `sender:`. `FIELDS` follows SEARCH §8's table, `flag` before `is`, as its doc says.
- src/sql/search.rs, spec/queries/search/init_index_meta.sql: `search-draft-18`, `FORMAT_DRAFT` 18; an index of search-draft-17 is rebuilt.
- tests/search.rs: queries.json `v` 6, each hit a case names under `matched` compared with `PimdirSearchHit::matched`; refresh.json `v` 7, a query stating a `limit` or `ordered` asked by `paged` in its order after the cursor the query its `page` names ended on, on the refreshed and the rebuilt index and under every driver, which end it on one cursor; `every_extraction_vector_answers` over vectors/search/extraction.json, a new suite `extraction` of tests/pending_vectors.txt.

## Guards

- The vector cases `kind:mail date:2w..2w` and `linked:sender:` fail on 769dfba (hits for the first, the link id `sender:` accepted for the second), and src/client/search/compile.rs `relative_dates_resolve_against_now` gains `2w..2w`; src/search/query.rs `a_linked_value_reads_a_role_only_when_it_names_one` gains the refusal.
- Every new case of queries.json (51), refresh.json (11 cases, 25 steps) and extraction.json (7 cases, 79 queries) was computed from SEARCH.md and the store before io-pimdir answered it, and io-pimdir answers each alike, the two readings above aside. The one disagreement went the other way: pimdir's checks/vectors.py modelled every removed row as retained, where io-pimdir purges one whose identity another collection of the account holds live with its body, as STORAGE §11 says; the checker was fixed.

## Spec

[search](../spec/search.md): *The vectors are the bar* names queries.json `v` 6, refresh.json `v` 7 and its pages, and extraction.json; the query parse requirement states the single point and the refused `linked:` value; search-draft-18 throughout. spec/REVISION still names bd36f0e: tests/spec_drift.rs `the_vendored_copy_is_the_spec_byte_for_byte` fails on the vendored files until the pimdir commit exists and REVISION names it.
