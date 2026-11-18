---
cairn: change
id: a-page-sorts-its-own-hits
status: landed
created: 2026-11-18
---

# A page sorts its own hits

## Why

CLI family analysis of 2026-11-15, PIM-10, re-checked at 117b5e7: every page of a broad query sorted the whole hit set (100,000 messages: 1.18 s for a page whatever its offset), and every overlaid read read the conflicts of every item a pending `update`, `remove` or `move` names in the collection, about 15 statements each, before showing a page of a few. Paging became a cursor on 2026-11-17 (0.16.0), so the page can be selected rather than sliced. No format change: what an answer holds is the same.

## What

- src/client/search.rs: `search` keeps the hits following the cursor, selects the first `limit` (`select_nth_unstable_by`) and sorts those alone; `threads` and the tag plan still sort the whole answer, which they walk whole.
- src/client/reader.rs: the overlay reads the conflicts of an item only when it folds that item and an `update`, a `remove` or a `move` pending on it meets them (`folded`), no longer for every such row of the collection when the overlay is built.
- Left: the statement per hit `--threads` runs (`hit_thread`), which needs a canonical statement reading the threads of a slot of hits, so a format change; the evaluation of the whole query per page, which SQL cannot page under the `name` order's fold.
