---
cairn: change
id: an-expansion-bounded-by-its-instances
status: landed
created: 2026-11-18
---

# An expansion bounded by its instances

## Why

CLI family analysis of 2026-11-15, PIM-15: one 570-byte event stalled the indexer, its `COUNT` walked instance by instance from a `DTSTART` of 1914 (22 s in release on d7cfe5e, 205 s in debug). The owner ruled on 2026-11-18 (D7 (a) of pimdir's the-draft-14-lines) that SEARCH §7 bounds the instances an expander walks, 10,000,000 from `DTSTART`, folded into search-draft-14 before its tags (pimdir cd77e7b).

## What

- src/search/occurrence.rs: `INSTANCES_MAX`; `examined` walks each rule's instances from `DTSTART` up to the horizon's end and its first unexamined period, stopping at its `COUNT` or at the bound, whose next instance is the cut; the windowed walk starts no later than the cut and ends at it, so a `COUNT` is never counted past it.
- spec/ re-vendored at pimdir 2997103: search store item 35, its window and the `cut` cases.
