---
cairn: log
change: reader-role
date: 2026-08-27
---

# The third role got a handle, and the queue got read

The format names one owner, any number of producers and any number of readers (spec §8), and the crate shipped two handles: `open_read_only` returned a `PimdirStore`, the same type that drains the queue, sweeps the objects and purges the trash. A consumer that only reads held a handle that could destroy the store, and "it never calls those" was the only thing stopping it. Capabilities `store` and `cli` moved.

## What landed

- **`PimdirReader`** (capability `store`), opening a store read-only, taking no lock, carrying the read surface and nothing else. The reads moved to it wholesale: collections, accounts, items and their pages, `get_item`, `seq_for_link`, link and object placements, the retained reads, generations, the queue reads, and the diagnostics behind `pimdir check`. `PimdirStore` holds one and dereferences to it, so both roles run the same statements and neither can drift from the other. `open_read_only` is deprecated.

- **The §15.4 overlay**, on `with_pending()`. The whole pending queue is folded in global row order into what it changes about one collection: `set-flags` and `update` restate an item, `remove` and `move` take it out, `move` and `copy` bring it in. Every one of them addresses an existing item, and a `seq` follows its link id store-wide (spec §9.1), so an arrival is read from the collection whose row still holds it and keeps the id it already had. Nothing here invents an identifier.

  A queued `add` is not folded in. It has no `seq` until the owner applies it, and it is a request to create an item rather than one; `pending_creates` and `count_pending_creates` report those apart, for a consumer to surface its own way. Parked rows never overlay: the error says the row will not be applied without an operator, and reading it as pending would promise otherwise.

  The choice is made at construction, never per call, so one handle cannot answer two ways about one collection. A page merges its arrivals before cutting back to the limit, which keeps the cursor total: an arrival past the last item comes back on the next page instead of being lost. A staged removal shortens a page, so a caller pages until an empty page rather than until a short one, which the doc comment says outright.

- **`PimdirStore::cancel_action(dir, id)`**, the scoped owner operation. Cancelling is an owner write (spec §15.5) and the only retraction a queued create has, the other kinds being retracted by their inverse; reaching it used to mean opening the handle that can also drain and sweep. It opens as owner, cancels one row, and has released the lock by the time it returns. It refuses a store it cannot find rather than creating one, and a store another process owns fails fast.

- **The CLI reads through the reader** (capability `cli`): `StoreFlags::read()` returns a `PimdirReader`, and `locate`, `retained` and the export dumps take one. Without the overlay, deliberately: an operator inspects the store as it stands and reads the queue through `queue list`, where a pending row is a row rather than a fact about an item.

## What did not land

No format change. §15.5 stays an owner write and the producer gained no self-cancel.

## A note on the refactor

Moving the fields onto `PimdirReader` cost the disjoint-field borrows the write path relied on: `self.store.conn` and `self.store.blobs` reach through `Deref` now, which borrows the whole handle, so the three write batches name `self.store.reader.conn` and `self.store.reader.blobs` instead. A field path splits where a deref cannot.

## Tests

tests/overlay.rs, ten cases: a staged flag visible only through the overlaying reader and absolute rather than additive; two actions on one item folding in append order; a staged removal leaving both the listing and the count; a move leaving one collection and entering the other with its id intact; a copy entering the target while staying in the source; a descending page staying ordered and total across two arrivals, checked one item per page so a repeated cursor would show; a create counted and never listed; a parked action changing nothing; the scoped cancel refused while the lock is held elsewhere, then succeeding and releasing; and a cancel that creates no store at the path it cannot find.

The contention case holds the lock through a file description of its own, the way tests/owner_lock.rs does: one process owning a store twice is still one owner, so an in-process second handle would have shared the lock rather than been refused it.

114 tests pass, clippy clean.
