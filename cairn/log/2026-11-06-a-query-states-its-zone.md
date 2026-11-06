---
cairn: log
change: a-query-states-its-zone
date: 2026-11-06
---

# A query states its zone

Follows pimdir's change of the same id (pimdir cairn/log/2026-11-06-a-query-states-its-zone.md, uncommitted there when this landed): SEARCH §8.1 now says a span runs from the first instant of its first day in the client's zone, and a client should state the zone its indexer states as `floating_zone`.

## What landed

- `PimdirSearchClock::zone` (`Option<String>`, `None` for UTC). `compile::bounds` reads a year, a month, a day, `today` and `yesterday` from the zone's start of day (jiff `Date::to_zoned` then `Zoned::start_of_day`, so a midnight a transition skips starts after the gap); a day past jiff's range keeps its UTC midnight. Relative amounts are unchanged instants. The temporal coverage and `cut_before` read the same bounds. An unknown zone is `PimdirError::IndexZone` (`index-zone-unknown`), its message widened to cover a query's dates; no new variant. `PimdirSearchClock` and `PimdirSearchOptions` lose `Copy`. `default()` is still UTC, so existing callers answer as before.
- `PimdirIndex::floating_zone`, read from `load_index_meta`, on `open_query` handles too; `erase` reads its zone through it.
- The CLI's `search`, `count` and `tag` take `--zone ZONE`, defaulting to the index's floating zone after the refresh.
- spec/vectors/ re-vendored from pimdir's working tree (four fixtures, store.json, queries.json at `v` 3, its schema, manifest.json, README.md). tests/search.rs answers the cases stating no zone on the UTC index, then refreshes the index in each zone a case states and answers those through an `open_query` handle whose `floating_zone` builds the clock, a case's `now` replacing the file's; a case's pending-list name carries its zone and now.
- Proven failing: with `time_zone` forced to UTC, seven zoned vectors fail (2026-12-31, 2027-01-01, 2026-12, 2027, both `today`, `yesterday`).
- Tests: compile.rs `calendar_spans_start_at_the_zone_midnight`, `a_day_a_transition_moves_is_23_or_25_hours`; tests/search.rs `a_query_reads_the_index_zone_and_refuses_an_unknown_one`; tests/search_cli.rs `a_query_reads_its_days_in_its_zone`.
- Moved: [search](../spec/search.md) ("Dates resolve against a clock in a zone", renamed from "Relative dates resolve against a clock"; "The vectors are the bar").

## Open

- spec/REVISION still names pimdir 1d34d2f, so tests/spec_drift.rs reports the re-vendored files until pimdir's change is committed and the revision moved to it.
