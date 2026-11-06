---
cairn: delta
change: a-query-states-its-zone
---

## MODIFIED Requirements

### Requirement: Dates resolve against a clock in a zone
Renamed from "Relative dates resolve against a clock": a query's calendar spans are read in `PimdirSearchClock::zone`, UTC when `None`; `PimdirIndex::floating_zone` reads the index's zone; the CLI takes `--zone`.

### Requirement: The vectors are the bar
queries.json at `v` 3, 179 cases, the zoned ones asked on an index refreshed in their zone.
