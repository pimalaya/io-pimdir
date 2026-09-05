---
cairn: change
id: overlay-page-is-total
status: landed
created: 2026-08-27
---

# An overlaid page came back short in the middle of a collection

## Why

`reader-role` shipped the §15.4 overlay with a caveat in its doc comment: a page shortened by a staged removal comes back below its limit, so a caller paging until a short page had to page until an empty one instead. Writing the first consumer showed the caveat was a defect, not a documented cost.

A page is the store's unit of paging, and every caller reads it the same way: ask for `limit`, stop when fewer come back. Himalaya's whole-collection scan is written exactly that way, and so is the reference paging in this crate's own docs. Under the overlay one staged removal ends such a scan early and silently, losing every item past that page, which is the failure the overlay exists to prevent: the frontend stages a delete and the next listing drops unrelated messages.

Documenting it does not help. The caveat is a contract change nobody reads for, and it would have to be honoured by every consumer of every read, forever, to buy nothing.

## What

The fold over-reads. At most one row per removing action can be dropped from a page, and the pending queue is already loaded to build the overlay, so the statement is asked for `limit` plus that many rows and the result is cut back to `limit` after the fold. A page is then short only where the collection really ends, staged removals or not.

The cost is bounded by the queue, not by the collection: the over-read is the number of pending removals, which a sync drains.

## Not in scope

Arrivals already worked: a `move` or a `copy` into the collection is merged before the cut, so an arrival past the last item comes back on the next page rather than being lost.
