---
cairn: delta
change: a-narrowed-round-proves-only-what-its-enumeration-lacks
---

## MODIFIED Requirements

### Requirement: A narrowed listing proves only what its enumeration lacks
A round whose last page states `narrowed` SHALL find absent only a member in its scope whose handle that page's `enumerated` lacks, whatever its `date`, and none when the page hands no `enumerated`, counting what it left unlisted in `PimdirSyncReport::unlisted`. The last page loads `LastPage { narrowed: false, enumerated: None }` and the engine applies the page's proof itself. Replaces: "A narrowed listing proves only what its filter could list", where a member dated before `round_started_at` was absent whatever the enumeration, the store bound with the page's narrowing, and a narrowed round its last page opened was opened in a write of its own first.

#### Scenario: A band narrowed by arrival and an old `Date` received late
- GIVEN a band narrowed by arrival, a message dated 1 January received in November that its pages miss and its enumeration holds
- WHEN the last page lands
- THEN it stays as it was (vectors/sync/160, 161)

#### Scenario: A narrowed round handing no enumeration
- GIVEN three bound members a narrowed round without an enumeration did not list, one dated before it opened
- WHEN the last page lands, in the run that opened the round or a later one
- THEN none is dropped, the round closes and `unlisted` counts three (vectors/sync/107; tests/scoped_sync.rs `a_resumed_narrowed_round_handing_no_enumeration_drops_nothing`)

### Requirement: Absence means deleted in scope only
`list_unstamped_bindings` SHALL always be bound with `:narrowed` 0, `:enumerated` 0 and an empty `handles` slot, the in-memory rule agreeing with it; what a narrowed listing proves is applied after it.

### Requirement: A capped listing proves no absence
`PimdirSyncReport::unlisted` SHALL count the based bindings in scope a capped round, or a narrowed one handing no enumeration, did not list.

### Requirement: A narrowed listing says so
A filter misses an undated member, one whose `Date` runs ahead of its arrival and one received long after its `Date` past a ceiling; the engine finds a member of a narrowed round absent only on the enumeration's word, and none without it.
