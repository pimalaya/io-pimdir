---
cairn: change
id: a-query-states-its-zone
status: landed
created: 2026-11-06
---

# A query states its zone

## Why

pimdir SEARCH §8.1 reads a query's years, months, days, `today` and `yesterday` in the zone the client states, and a client should state the one its indexer reads floating times in (§7). The crate could take neither: `PimdirSearchClock` carried `now` alone and `compile::bounds` cut days at UTC midnight, and a query client had no read of `index_meta.floating_zone`. calendula lists a user's days and needs both (its cairn/changes/pimdir-search/proposal.md, "Depends on").

## What

- `PimdirSearchClock::zone`, an IANA name, `None` for UTC; spans from the first instant of a day in the zone (jiff), relative amounts unchanged; an unknown zone is `PimdirError::IndexZone`.
- `PimdirIndex::floating_zone`, on every handle.
- The CLI's `--zone` on `search`, `count` and `tag`, the index's floating zone by default.
- pimdir's new vectors (queries.json `v` 3, cases stating `zone` and `now`) vendored and run.
