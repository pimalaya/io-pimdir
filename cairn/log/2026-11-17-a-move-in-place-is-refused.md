---
cairn: log
change: a-move-in-place-is-refused
landed: 2026-11-17
---

# A move in place is refused

CLI family audit of 2026-11-15, PIM-14, open at dcf49e2: nothing compared a queued `move`'s target with the row's own collection, so the item was tombstoned and a create staged under `dup:<id>#...` beside it; for a message held without its body that create could never deliver and the tombstone pushed a plain `Remove`, a server-side delete nobody asked for. himalaya guards it in its own backend; any other producer did not.

## What landed

- src/mutate.rs: `PimdirMutateError::MoveInPlace` (`move-in-place`), a `Move` into the collection its placement is in staging nothing. The drain runs the mutation for a queued `move`, so the row parks with it.
- Tests: src/mutate.rs `a_move_into_its_own_collection_is_refused`, tests/queue.rs `a_queued_move_into_its_own_collection_parks` (applied on dcf49e2).
- Left for the owner: the rule in STORAGE §15.3 and its vector (draft-14), and whether `enqueue` refuses such a row up front, which needs a store error code.

## Capabilities moved

mutate ("A move into its own collection is refused", added).

In the working tree on io-pimdir dcf49e2 (0.15.1), uncommitted, no version bumped; CHANGELOG.md [Unreleased] names it.
