---
cairn: tasks
change: overlay-page-is-total
---

- [x] The fold over-reads by the number of pending removals and cuts back after
- [x] `PimdirPending::removals`, the bound on what a page can lose
- [x] The page callers pass a fetch closure rather than a fetched page
- [x] The `with_pending` caveat goes: a page is short only at the end
- [x] Test: a page of two over a collection whose first item is staged for removal comes back full, and the next page is empty
