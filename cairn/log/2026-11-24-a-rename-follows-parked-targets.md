---
cairn: log
change: a-rename-follows-parked-targets
landed: 2026-11-24
---

# A rename follows parked targets

Left from the submit rename fix of the same day (cairn/changes/the-live-night-findings, F3; the owner's standing instruction, the recommended option): `rename_queue_targets` retargeted pending rows alone (`WHERE error IS NULL`), so a move, a copy or a superseding add parked before a rename of its target, retried after it, named an id gone and parked again on "target collection has no declared kind". A queued `submit`'s `copy` already followed on parked rows.

pimdir STORAGE §14 already has `queue.target_collection` follow the rename with no word of pending rows; no part's text moves.

## What landed

- spec/queries/storage/owner/rename_queue_targets.sql: every row naming the old target follows, pending or parked.
- tests/invariants_queue.rs `a_parked_move_retried_after_a_rename_lands_in_the_renamed_target`: a move into Archive parked, Archive renamed, the row retried and drained; failed before ("applied: 0, parked: 1").

## Spec

[store](../spec/store.md): *A collection can be renamed without losing its contents* and *A queued action's target is a column a rename follows* say pending or parked; new scenario *A parked move retried after a rename*.
