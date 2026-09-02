---
cairn: log
change: unlinked-upsert-keys-onto-its-binding
date: 2026-09-02
---

# A reprobed handle is the item it is already bound to

The store's half of the duplicate-link-id family, and the last write that could mint a `dup:` key for a copy nobody holds.

An `UpsertPlacement` carrying no `link_id` was staged in the residual unconditionally, on the reasoning that an unlinked placement is a fresh probe whose identity a later `Meta` upgrade will resolve. That is true of a handle the store has never bound and false of one it has: the identity the write did not carry is the one the store is already holding for that handle.

io-replica writes exactly that. When a remote edit lands on an item deleted locally, the update wins and the tombstone is replaced by a fresh pull of the new revision, which is `pull_add` building a probed placement for the same handle with `link_id: None` (`sync.rs`, the local-tombstone branch). The store filed it beside the tombstoned item that handle binds, so the next `load` answered with two placements for one handle. `ReplicaUpgrade` collects placements into a map keyed by handle, so it fetched the handle twice, read one identity twice and minted `dup:<hint>#<handle>` for the second read, leaving one source handle bound to two hub items that no later sync can converge. One item that was never duplicated on any side, permanently split.

The rebind guard was blind to the same write. It begins by skipping every placement carrying no link id, so an unlinked upsert reached neither the guard nor the link set the batch folds into, and the hub the write diffed against did not contain the item the handle was bound to.

## What landed

- **`apply_ops` resolves an unlinked upsert before it routes it** (capability `store`): `(collection, source, handle)` through `LINK_FOR_HANDLE`, the statement the drop arm beside it already ran, served by `bindings_by_handle`. A handle a binding holds folds through the hub under that link id; a handle no binding holds stages unlinked exactly as before, which is a first enumeration and every genuinely new item after it. One indexed lookup on a write that previously did none.

- **The floor now covers it.** With a link id on the placement, the batch guard refuses a batch claiming one identity under two handles even when one of the two carries none, and the batch's affected-link set includes the item, so the fold diffs against the rows it is actually changing. A refused batch also leaves no residual row behind any more: the resolution happens before the routing, so nothing is inserted into the in-memory staging that the rolled-back transaction cannot undo.

- **`link_for_handle` is the one copy of that lookup**, used by the new resolution, by `batch_links`'s drop arm and by `load`'s handle scope, which had two transcriptions of the same statement between them.

Nothing else moved: no schema change, no new state, no change to what the engine mints, and `write_rekeyed` is untouched, a rebuild carrying link ids on every placement.

## Verification

- `tests/unlinked_upsert.rs` is the regression, written first and red on three of its four cases: a bound handle reprobed answers `load` with two placements; the resurrection shape (clean, then tombstone, then probe) answers with two, one of them still a tombstone; and the rebind guard's refusal leaves the residual row behind. Green after, with the fourth case, a probe of an unbound handle staying unlinked, green throughout as the neighbour it guards.
- 142 tests green (15 lib, 127 integration), `cargo clippy --all-features --all-targets` clean, `cargo fmt`.
- The neighbours the change could have broken all hold: `tests/duplicate_link_id.rs` (a source legitimately holding one identity twice is still two items, and a colliding write is still refused), the roundtrip and hub suites, retention and revival, the refcount property suite, and the spec-fidelity suite against the sibling pimdir checkout.

Capabilities moved: `store`.
