---
cairn: delta
change: an-expansion-bounded-by-its-instances
---

## ADDED Requirements

## MODIFIED Requirements

### Requirement: Occurrences are bounded and a cut is reported
An item SHALL write at most 10,000 occurrences (`search::occurrence::OCCURRENCES_MAX`), the first by start then end of those overlapping the horizon, SHALL examine at most 1,000,000 periods of a rule (`PERIODS_MAX`), a period being `INTERVAL` times `FREQ` from the one holding `DTSTART`, a period before the horizon counting, and SHALL walk at most 10,000,000 instances of a rule (`INSTANCES_MAX`), each start the rule yields from `DTSTART`, an instance before the horizon counting, whatever `COUNT` the rule states beyond them (SEARCH §7). A rule whose 10,000,001st instance, or else whose first unexamined period, starts before the horizon's end, and which neither its `UNTIL` nor its `COUNT` ends before it, is stopped there, its occurrences from that instant on not written. An item a bound stops SHALL record its cut with its occurrences (`set_cut`): the earliest of its 10,001st occurrence's start, its first unexamined period's and its first instance not walked, `NULL` otherwise. The walk of a rule's instances stops at the horizon's end, and the windowed walk starts no later than the cut and ends at it, so no rule costs more than the instance bound. A dated answer's coverage SHALL count the items cut before its range's end (`cut_before`). Departure: the walk is ical-rs's, which skips ahead over periods a `BYxxx` part rules out; the period bound is computed from the rule's `DTSTART`, `FREQ` and `INTERVAL` rather than counted, and the 10,001 first occurrences of each timeline are taken in identity order before the item's are ordered, so an override moving an instance from past them to before them is left out.

#### Scenario: An hourly series and a sparse one
- spec/vectors/search store items 27 (`FREQ=HOURLY`, 10,000 occurrences, cut at 2028-03-23T16:00:00Z) and 28 (`FREQ=SECONDLY` limited to 1 August at midnight, its `DTSTART` alone, cut at its millionth second), their two occurrence windows and the four cases stating `cut` (tests/search.rs `every_search_vector_answers`); src/search/occurrence.rs `one_item_writes_at_most_the_bound_and_is_cut_at_the_next`, `a_rule_is_examined_for_at_most_the_period_bound`

#### Scenario: A rule filling every period
- spec/vectors/search store item 35 (every second from 2025-04-26T19:13:20Z under `COUNT=4000000000`, cut at 2025-08-20T13:00:00Z, its 10,000,001st instance), its window and the case `kind:event date:2025-08-20` (tests/search.rs `every_search_vector_answers`, failing on io-pimdir a8f6cda); src/search/occurrence.rs `a_rule_is_walked_for_at_most_the_instance_bound`, the PIM-15 rule from 1914 cut at its 10,000,001st second

## REMOVED Requirements
