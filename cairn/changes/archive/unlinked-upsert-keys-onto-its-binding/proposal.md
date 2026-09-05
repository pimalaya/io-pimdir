---
cairn: change
id: unlinked-upsert-keys-onto-its-binding
status: landed
created: 2026-09-02
---

# An upsert with no link id is not automatically a new item

## Why

A probed placement carries no `link_id` because its identity has not been read yet, so the store stages it apart from the hub until a `Meta` upgrade names it. That is right for a handle the store has never seen, and wrong for a handle a binding already holds: such a handle already names an item, and the identity the write did not carry is the one the store is holding for it.

io-replica produces exactly that write. A remote edit of an item deleted locally is pulled rather than pushed, and the pull is `pull_add`, which builds a fresh probed placement for the same handle with no link id (`sync.rs`, the local-tombstone branch). The store filed it in the residual beside the tombstoned item the handle is bound to, so the next `load` answered with **two placements for one handle**.

That answer is the store's half of the duplicate-link-id family. io-replica's `ReplicaUpgrade` collects placements into a map keyed by handle, so it fetches the handle twice, reads one identity twice, and mints `dup:<hint>#<handle>` for what it takes to be a second copy. One source handle then binds two hub items that no later sync can converge, out of one item that was never duplicated anywhere.

The rebind guard could not catch it either: it skips a placement carrying no link id, so the collision was invisible to the one check whose job is to see it.

## What

- **The unlinked upsert resolves its handle first.** `write` resolves `(collection, source, handle)` through `LINK_FOR_HANDLE`, the way a drop already does, and folds the placement through the hub under the link id it finds.
- **The residual keeps its purpose.** A handle no binding holds still stages unlinked, which is the first enumeration of a collection and every genuinely new item after it.
- **The rebind guard and the batch's link set see it.** Both read the placement's link id, so a resolved upsert is subject to the floor and its item is part of the hub the batch folds into.

## Scope / non-goals

- **No new state, no schema change.** One lookup on a write that used to do none, served by `bindings_by_handle`.
- **No change to the engine's minting.** A source that genuinely holds one identity twice still reaches the store under two handles and is still stored as two items.
- **`write_rekeyed` is unchanged**: a rebuild carries link ids, so nothing in it is unlinked.
