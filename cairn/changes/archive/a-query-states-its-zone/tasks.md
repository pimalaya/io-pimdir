---
cairn: tasks
change: a-query-states-its-zone
---

- [x] src/client/search.rs: `PimdirSearchClock::zone`, `PimdirIndex::floating_zone`, the zone through `compile` and `coverage`
- [x] src/client/search/compile.rs: `bounds` and `span` in a zone, unit tests for a UTC midnight and a transition day
- [x] src/cli/search.rs: `--zone` on `search`, `count` and `tag`
- [x] spec/vectors/ re-vendored from pimdir's working tree; tests/search.rs asks zoned cases on an index refreshed in their zone through `open_query`
- [x] A zoned case fails with the zone ignored (seven cases)
- [x] spec/REVISION moved to pimdir 8c16bb1, the commit holding the vectors
- [x] CHANGELOG; spec search.md folded; log
