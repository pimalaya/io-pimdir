---
cairn: change
id: a-hostile-interval-ends-the-walk
status: landed
created: 2026-11-17
---

# A hostile interval ends the walk

## Why

CLI family audit of 2026-11-15, PIM-1 (HIGH) and PIM-17 (LOW), re-checked open on 2026-11-16 at c40e01d and unchanged at dcf49e2. One `VTIMEZONE` observance with `RRULE:FREQ=MINUTELY;INTERVAL=4611686018427387904` made `ZoneRule::onsets` multiply past `i64` (src/summary/calendar.rs, `step * self.interval`): a panic in debug, a wrapped zero and a division by zero in release, so the derivation aborted calendula, the `pimdir` binary on `gc` and a sync engine pulling the resource. Its sibling in the search expansion, `occurrence::unexamined`, multiplied an unbounded `INTERVAL` too: a panic in debug, and in release a wrapped period that cut every occurrence.

## What

- src/summary/calendar.rs: checked step, onset and `UNTIL` arithmetic in `ZoneRule::onsets`; an overflow ends the walk as the period bound does, no cap on `INTERVAL` (the audit's amended direction).
- src/search/occurrence.rs: `unexamined` computes the first unexamined period with checked arithmetic and answers `None` for any period past 9999-12-31.
- The expansion's yield changes for such a rule, so `EXTRACTOR` moves to `io-pimdir 2` with a-comment-closes-on-any-run-of-dashes.
- Tests: src/summary/calendar.rs `an_observance_rule_stepping_past_every_instant_ends_its_walk`, tests/zone_rule_property.rs, src/search/occurrence.rs `a_rule_stepping_past_year_9999_is_not_cut`.
- Left: the conformance vector with an overflowing `INTERVAL` the audit asks for is pimdir's (draft-14).
