---
cairn: change
id: reader-role
status: landed
created: 2026-08-27
---

# A reader role, and a cancel that does not hand out the owner

## Why

The format names three roles (SPEC §8): the one owner that writes, the producers that enqueue, and the readers that take no lock. The crate ships two types. `open_read_only` hands back a `PimdirStore`, the same type that carries `drain_collection`, `collect_garbage`, `purge` and `fail_action`, so "Himalaya is not the owner" is a statement about which methods a consumer chooses to call, not about the handle it holds. Himalaya's `pimdir-producer-reader` made that promise; nothing in the type system keeps it, and the next consumer will not even know it was made.

That missing type is also why the store reads as shapeless from outside. It is one format with three roles, and with only two of them expressed, the same handle looks like a database, a sync backend, a queue and a garbage collector at once.

**Nothing implements §15.4.** The spec invites a reader to overlay a collection's pending actions on its projection for read-your-writes, and `PimdirProducer::pending_actions` hands out the rows, but the fold from committed items plus actions to a projected view exists nowhere. Every consumer that wants read-your-writes writes its own, and each one will drift from what the owner's drain actually does. Himalaya has the visible symptom today: a staged flag is invisible until Neverest runs.

**Cancel forces the owner handle on a non-owner.** §15.5 makes cancelling an owner write, and it is the only retraction a queued action has. A consumer that wants to offer it has to open a full owner handle to reach `drop_action`, which is precisely the handle it must not hold.

## What

- `PimdirReader`: a read-only handle, taking no lock, carrying the read surface and nothing else. No drain, no sweep, no purge, no retention write, no enqueue. The separation is by type, not by SQLite refusing a write at run time.
- The §15.4 overlay lives on the reader. It folds a collection's pending actions over the committed items in row `id` order, covering the kinds that address an existing `seq`: `set-flags`, `remove`, `move`, `copy`, `update`. All of them keep the item's public id, since a `seq` follows the link id store-wide (§9.1), so the overlay never invents an identifier.
- A pending `add` is not projected as an item. It has no `seq` until the owner applies it, and it is a request to create a message rather than a message. The reader reports pending creates separately, as rows and as a count, for a consumer to surface its own way.
- Parked rows never overlay. A parked action asserts nothing about a future state, and showing it as pending would promise work that will not happen without an operator.
- The overlay is chosen when the reader is built rather than per call, so one handle cannot show two different pictures of the same collection.
- A scoped owner operation for cancel: an associated function taking the store directory and a row id, which opens as owner, cancels, and releases the lock before returning. A consumer reaches §15.5 without ever naming a type that can drain. Fail-fast is kept: a store another process owns is `PimdirError::Owned`, not a wait.

## Not in scope

No format change. §15.5 stays an owner write and the producer gains no self-cancel: for the kinds that address an existing item the retraction is the inverse action, `set-flags` being absolute rather than a delta, and for a submission intent SMTP offers no undo either. The one case with no inverse is a queued create, and the operator hatch (`pimdir queue cancel`, and now the scoped operation) is what answers it.

`PimdirStore` keeps its own read methods; the owner reads in order to merge. Both roles run the same statements over a `&Connection`, so the projection is one implementation seen through two handles.
