---
cairn: log
change: an-expansion-bounded-by-its-instances
landed: 2026-11-18
---

# An expansion bounded by its instances

pimdir cd77e7b folds the owner's ruling D7 (a) of 2026-11-18 into search-draft-14: SEARCH §7 bounds the instances an expander walks at 10,000,000 from `DTSTART`, the cut at the first one not walked. CLI family analysis PIM-15: a 570-byte event walked every instance its `COUNT` named.

## What landed

- src/search/occurrence.rs: `INSTANCES_MAX`. `examined` walks each rule's instances up to the horizon's end and its first unexamined period, ended by its `COUNT` or cut at its 10,000,001st instance; the windowed walk starts no later than the cut and ends at it, so a `COUNT` is never counted past the bound.
- spec/ and spec/REVISION at pimdir 2997103: search store item 35, its window and five cases stating `cut`.
- Tests: src/search/occurrence.rs `a_rule_is_walked_for_at_most_the_instance_bound` (the PIM-15 rule from 1914, `COUNT=4000000000`, cut at its 10,000,001st second, and one ended by a `COUNT` of 100, not cut); the search vectors, which fail on a8f6cda.
- Timed, release build, the PIM-15 event (`FREQ=HOURLY`, every minute and second, from 1914-06-01): 138 ms under `COUNT=4000000000`, where d7cfe5e took 22 s; the same rule without a `COUNT` is cut at the same instance (113 ms), where it wrote 10,000 occurrences before.

## Capabilities moved

search ("Occurrences are bounded and a cut is reported", modified).

In the working tree on io-pimdir a8f6cda, uncommitted, no version bumped; CHANGELOG.md [Unreleased] names it.
