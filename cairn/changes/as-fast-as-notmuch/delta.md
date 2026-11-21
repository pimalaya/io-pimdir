---
cairn: delta
change: as-fast-as-notmuch
---

## ADDED Requirements

### Requirement: A query costs what it answers
A search, a count and a threaded page SHALL read the store and the index in proportion to the smallest set among the query's terms and to the page asked, never to the store's live items: the compiled statement SHALL be driven by one term's set and probe the others, and a page SHALL be cut before every hit is read when the order allows it.

#### Scenario: A rare word in a large store
- GIVEN the bench's 517k Enron store
- WHEN `kiwi` (35 hits) is counted and paged
- THEN each answers no slower than notmuch 0.40's `notmuch count kiwi` and `notmuch search --limit=50 kiwi` on the same mail (198 ms and 213 ms against 3 ms and 4 ms on 2026-11-21)

#### Scenario: Page 1 of everything
- GIVEN the same store
- WHEN `kind:mail` is paged 50 newest first, and with `--threads`
- THEN each answers no slower than notmuch's (3,045 ms and 13.8 s against 133 ms and 141 ms on 2026-11-21)

### Requirement: Coverage costs what moved
An answer's coverage SHALL be computed once per query and SHALL read, per collection, only what changed since it was last read: no statement per collection per query, and the unindexed count a seek on the change stamp.

#### Scenario: Thousands of collections
- GIVEN a store of 3,311 collections
- WHEN a query scoped to all of them is answered twice with no write between
- THEN the second answer's coverage reads no collection's rows (69 ms per answer on 2026-11-21)

### Requirement: An import is as efficient as notmuch's
A first sync writing every body SHALL take no more wall time than `notmuch new` over the same messages on a quiet machine, the bench reporting CPU time and bytes written per message beside it.

#### Scenario: The 517k tier
- GIVEN the bench's Maildir of 517,401 Enron messages
- WHEN it is synced and indexed through the bench's connector
- THEN the wall time is at most notmuch's (2,527 s against 719 s on 2026-11-21)

## MODIFIED Requirements

## REMOVED Requirements
