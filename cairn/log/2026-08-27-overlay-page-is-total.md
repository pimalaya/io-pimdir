---
cairn: log
change: overlay-page-is-total
date: 2026-08-27
---

# A page is short only where the collection ends

`reader-role` landed the §15.4 overlay with a caveat: a page shortened by a staged removal came back below its limit, and a caller paging until a short page had to page until an empty one instead. Writing the first consumer showed the caveat was a defect. Capability `store` moved.

## What landed

- **The fold over-reads.** At most one row per removing action can be dropped from a page, and the pending queue is already loaded to build the overlay, so the statement is asked for `limit` plus `PimdirPending::removals()` rows and the result is cut back to `limit` after the fold. The over-read is bounded by the queue, which a sync drains, not by the collection.

- **The page callers pass a fetch closure** rather than a fetched page, since the fold now decides how many rows to ask for. `list_items` and `sorted_page` each hand `overlaid` a closure over their own statement and cursor; the unoverlaid path calls it once with the caller's limit and returns, so a reader without the overlay costs exactly what it did.

- **The caveat is gone** from `with_pending` and from the spec requirement. A page is short only where the collection ends, staged removals or not.

## Why it mattered

Himalaya's whole-collection scan stops when a page comes back below its batch size, which is how every consumer of a keyset page is written. One staged delete ended that scan early and silently, dropping every item past the page: the frontend stages a delete and the next listing loses unrelated messages, which is the failure the overlay exists to prevent. Documenting it would have made the contract change every consumer of every read had to honour, forever, to buy nothing.

## Tests

tests/overlay.rs: a page of two over a collection whose first item is staged for removal comes back full and in order, and the page after it is empty. The assertion that matters is the second one: a scan that pages until a short page has to reach the end, not stop at the removal.

115 tests pass, clippy clean.
