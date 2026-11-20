---
cairn: log
change: an-arrival-carries-its-earlier-rows
landed: 2026-11-20
---

# An arrival carries its earlier rows

Found by the pimdir backend tests of 2026-11-20: set-flags rows queued on #24 in INBOX, then a move to Archive; an overlaid read of Archive listed #24 without those flags. `PimdirReader::arrival` took the item's committed row from the collection it leaves and `arrived` folded only the target's rows, the source's being read for an item in a conflict alone. STORAGE §15.4 asks the overlay to fold a row as the owner would apply it, and the owner applies rows in order: its move (`PimdirMutation::Move`, src/mutate.rs `staged_copy`) carries the source's flags as the earlier rows left them.

## What landed

- src/client/reader.rs: `pending` walks the queue once and keeps the rows queued on items of other collections, so each arrival (`PimdirArrival`) carries the rows queued on its item in the source before its `move` or `copy`, the move last. `arrival` folds them over the committed row under the item's standing, the parking check of a conflicted move reading the same rows; the first `move` or `copy` into the collection wins, so a second move, which the owner parks, no longer stands for it. A row after a `copy` stays the source's; an `add` superseding the item in the source takes it out before it moves.
- Tests: tests/overlay.rs `an_arrival_carries_the_flags_queued_on_it_before_its_move_or_copy`, which failed on f93ecf3 (Archive listed `\Seen` for all three items).

## Capabilities moved

store ("A reader may overlay a collection's pending actions", "A queued move meeting a conflict parks", modified).

In the working tree on io-pimdir f93ecf3 (0.18.1), no version bumped; CHANGELOG.md [Unreleased] names it.
