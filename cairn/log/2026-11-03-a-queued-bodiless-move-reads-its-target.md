---
cairn: log
change: the-crate-follows-the-freeze-gaps
date: 2026-11-03
---

# A queued bodiless move reads its target

4e64216 had `PimdirSourceStore::mutate` state `PimdirMutate::syncs_target` from `collection_sources`, but not the drain: `stage_action` (src/client/producer.rs) built its `PimdirMutate` with the default `false`, so every queued `move` or `copy` of a placement holding no body parked as `Undeliverable`, the trash move of an undownloaded message among them. Found while moving the apps onto 4e64216. The drain now reads the target's sources as the direct path does.

Guarded by tests/queue.rs `a_queued_bodiless_move_applies_into_a_target_its_source_syncs`: a move into a target the source checkpoints applies, one into a target nothing syncs parks. It failed before the fix with `(0, 2)`, both parked.

`cargo test --all-features` 810 passed.
