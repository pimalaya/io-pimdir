---
cairn: delta
change: overlay-page-is-total
---

## MODIFIED Requirements

### Requirement: A reader may overlay a collection's pending actions
An overlaid page SHALL keep the meaning an unoverlaid one has: it comes back short only where the collection ends. A staged removal drops a row the statement returned, so the fold SHALL read past the limit by the number of pending removals and cut back to the limit afterwards, rather than returning a page shortened in the middle of a collection. A caller therefore pages until a short page, as it always did, and a whole-collection scan written that way does not end early.
