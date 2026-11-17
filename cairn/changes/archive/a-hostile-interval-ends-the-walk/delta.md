---
cairn: delta
change: a-hostile-interval-ends-the-walk
---

## ADDED Requirements

### Requirement: A zone rule stepping past every instant ends its walk
`ZoneRule::onsets` SHALL compute every step and onset of a `VTIMEZONE` observance's `RRULE` with checked arithmetic: an `INTERVAL` whose step, onset or `UNTIL` comparison overflows what an instant spells ends the walk there, as the bound of `ZONE_PERIODS_MAX` periods does (STORAGE Annex A.3), the onsets found before it standing. No cap SHALL be put on `INTERVAL`: a large legal value keys the event as a conforming writer's exact arithmetic does, the next onset lying past any wall time. A hostile resource, from a CalDAV server or a co-writer, never aborts the derivation, which runs on writes, on re-derivation and in a sync engine's pull.

#### Scenario: An overflowing interval keys the event as its DTSTART alone
- GIVEN an event zoned by its own `VTIMEZONE` whose observance states `FREQ=MINUTELY;INTERVAL=4611686018427387904`, `FREQ=SECONDLY;INTERVAL=9223372036854775807`, or an interval reaching `i64::MAX` exactly beside a negative `TZOFFSETFROM` and an `UTC` `UNTIL`
- WHEN it is derived
- THEN its sort key is the one the observance's `DTSTART` alone gives, and nothing panics (src/summary/calendar.rs `an_observance_rule_stepping_past_every_instant_ends_its_walk`; tests/zone_rule_property.rs over every `FREQ` the walk steps by, any `INTERVAL`, `COUNT` and `UNTIL`)

### Requirement: A rule stepping past year 9999 is examined whole
`occurrence::unexamined`, the start of a rule's first period past `PERIODS_MAX` (SEARCH §7), SHALL compute that period with checked arithmetic and answer `None`, every period examined, whenever it lies past 9999-12-31, however far past: an overflow or a day count beyond what a civil date spells never wraps to a cut in the past that would drop every occurrence, nor panics.

#### Scenario: A huge INTERVAL is not cut
- GIVEN an event starting 2026-08-01 under `FREQ=SECONDLY` to `WEEKLY`, each with `INTERVAL=200000000` and `INTERVAL=4294967295`
- WHEN it is expanded
- THEN its first occurrence is written and it records no cut (src/search/occurrence.rs `a_rule_stepping_past_year_9999_is_not_cut`)

## MODIFIED Requirements

## REMOVED Requirements
