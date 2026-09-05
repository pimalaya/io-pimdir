---
cairn: tasks
change: reader-role
---

- [x] `PimdirReader`, opening a store read-only and taking no lock
- [x] The read surface moves to it: collections, accounts, items, paging, `get_item`, `seq_for_link`, link and object placements, retained reads, generations
- [x] `PimdirStore` and `PimdirReader` run the same statements over a `&Connection`, one projection behind two handles
- [x] `open_read_only` deprecated in favour of `PimdirReader::open`
- [x] The CLI reads through the reader, without the overlay
- [x] The §15.4 overlay: pending actions folded over committed items in row `id` order
- [x] The fold covers `set-flags`, `remove`, `move`, `copy` and `update`, all of which keep the item's `seq`
- [x] A pending `add` is reported apart, never projected as an item
- [x] Parked rows never overlay
- [x] The overlay is chosen at construction, not per call
- [x] A scoped owner cancel: opens as owner, cancels one row, releases the lock before returning
- [x] Test: a queued `set-flags` shows on the overlaid read and not on the raw one
- [x] Test: two queued actions on one item fold in row order, the later winning
- [x] Test: a parked `move` leaves the item where it is
- [x] Test: a queued `add` raises the pending-create count and adds no item
- [x] Test: the scoped cancel refuses with `Owned` while another process holds the store
- [x] README feature list and lib.rs header name the third role
