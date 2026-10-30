---
cairn: change
id: rows-and-questions-draft-06
status: landed
created: 2026-10-30
---

# Follow storage-draft-06 and search-draft-06: rows from the summaries, questions from the index

## Why

pimdir ffe31a4 (cairn/log/2026-10-30-rows-from-summaries-questions-from-the-index.md) split what a list reads: a summary holds what a row shows, sorts by and what the store's references read; questions about content are the index's. The Android app's lists read and parse every body on each reload; the two places they still needed one are closed on that split, and this crate is the reference implementation of both parts.

## What

- **Vendored spec**: spec/ re-copied from pimdir ffe31a4 (tests/spec_drift.rs).
- **Annex A revision 2** (src/summary/calendar.rs, src/summary.rs): `PimdirTaskSummary` gains `created` (CREATED as an instant on A.3's rules, else DTSTAMP, else `None`) and `contacts` (every CONTACT in order: the address its `mailto:` ALTREP names, else its text, canonical; the text decoded as the name, none when the address was read from a text that is an addr-spec; an empty text with no such ALTREP files nothing), under a new `PimdirAddressRole::Contact` (`contact`). `upsert_task_summary` binds `:created`; the task readers read it.
- **Re-derivation** (src/client/schema.rs): `DERIVATION` 1 to 2; an owner opening a store derived under revision 1 pages `list_held_tasks`, writes `created` (`backfill_task_capture`) and the task's addresses (`replace_addresses`, `insert_address`, `stamp_item`) from each held body, and records revision 2 with `set_derivation` in the transaction writing the last page; until then the next open resumes it.
- **Index** (search feature): the card's `tel`, `adr`, `bday`, `photo` flags on `object` and its folded `CATEGORIES` in `object_category` at extraction; `has:phone|address|birthday|photo` through `holding`, `category:` on cards through `categorised` beside the task_summary seek; `PimdirIndex::occurrences_between` answering `occurrences_between`'s rows in order. An index of the earlier shape is rebuilt (SEARCH §2).
- **The owner's refresh** (SEARCH §4, SHOULD): stays the caller's, documented, since the store must not depend on the index (the `search` feature is optional and the owner path compiles without it).

## Out of scope

- `with:` reaching the `contact` role through the index's `people` field (left open by pimdir).
- conventions/gtd.md naming the new column (gtd-draft-02, pimdir's call).
