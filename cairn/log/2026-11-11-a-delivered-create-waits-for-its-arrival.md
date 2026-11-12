---
cairn: log
change: a-delivered-create-waits-for-its-arrival
date: 2026-11-11
---

# A delivered create waits for its arrival

Follows pimdir's change of the same id (pimdir cairn/log/2026-11-11-a-delivered-create-waits-for-its-arrival.md, uncommitted there when this landed): a create its source took before any listing named it, an add accepted with no handle or a relocation into it accepted, is delivered, its binding based under its provisional handle with a base naming nothing, and waits for the page naming its member instead of being withdrawn by a delete or a move staged on it.

## What landed

- `PimdirWriteOp::DeliverPlacement { collection, handle }`. The sync writes it when an add is accepted with no handle, and into the destination's collection when a relocating `Remove` is accepted (`Pending::Remove` carries the destination), only for a create a listing can land (`PimdirLinkId::is_landable`: a hint, or a hint minted over its own provisional handle). The client folds it into the collection's hub in the batch's order; `PimdirHub::absorb` bases the binding only while it holds none, so a create landed or withdrawn since is left alone.
- `PimdirHandle::is_provisional`, `PimdirPlacement::delivered`.
- `PimdirPushResult::relocated` (review of 2026-11-12): a connector states whether an accepted `Remove` carrying `to` relocated the member; only a relocation delivers, so a create whose relocation a connector carried out as a delete, the target already holding the identity (tests/engine/common.rs), stays pending and is pushed again rather than kept on the device alone. Vector 140 fails without it; tests/engine/identity_property.rs leaves out of its mirror check only a delivered create holding no body, and failed (Hydrate, Move, Sync, Move, Copy) before it.
- sync-draft-09 (the owner, 2026-11-12): `SYNC_VERSION`, so a store whose sync state sync-draft-08 wrote is refused (`sync-version`); README and cairn/spec/store.md name it; sync vectors read at format 4.
- The sync: the join's narrowing makes a delivered create no candidate while unlisted (no `Add`, `SetFlags`, `Update`, `Remove` nor drop); the opening round's absences, its mass-absence count and `stamped` leave it out; `take_pending_create` lands it; `land` keeps a tombstone's status and destination, so the run pushes its `Remove` against the listed handle. A delivered create waits whatever a round lists, counted in `PimdirSyncReport::waiting` by every delta or last page holding it (the review of 2026-11-11 replaced a re-pend on a closing round, which reopened the withdrawal and gave an app the arrival's markers). The vendored `destination_for_link` names a delivered create too, so an owner recording the delivery before the relocation leaves keeps the tombstone relocating.
- The upgrade fetches no provisional handle. The rekey carries a delivered create onto the member resolving to it, its base taking the staged body where no revision is reported, and otherwise leaves it untouched and uncounted.
- spec/ re-vendored from pimdir's working tree (deliver_binding.sql, list_unstamped_bindings.sql, origin_for_link.sql, vectors 20, 48, 92, 136 to 139, README, manifest).
- tests/vectors_sync.rs feeds no outcome back for a push its case states none for (vectors 93 and 111, "the remote reporting no outcome"): it read one as accepted with no handle, the same as unreported until that delivered the create.
- Proven failing: with spec/ re-vendored over the unchanged code, vectors 20, 92, 136 (then a round), 137 and 138 failed; 48, 136 as a delta and 139 were reworked or added after, and fail with the join's guard disabled. The identity property test (tests/engine/identity_property.rs) failed twice on intermediate versions, each a delivered create waiting for ever: one keyed by a key minted over another handle (hence `is_landable`), and one whose relocation the connector carried out as a delete, the target holding its identity under another key (now waiting, reported, and exempt from the mirror).
- Tests: src/sync.rs `an_add_accepted_without_a_handle_is_delivered` (was `..._stays_pending`), `an_accepted_relocation_delivers_its_destination`, `a_delivered_tombstone_waits_for_its_arrival`, `a_round_landing_no_delivered_create_keeps_it_waiting`, `a_landed_delivered_tombstone_pushes_its_remove`; src/placement.rs `a_key_minted_over_another_handle_is_landed_by_no_listing`; src/hub.rs `a_delivery_bases_a_pending_create_alone`; src/rekey.rs `a_delivered_create_is_carried_onto_its_member_or_left`.
- Moved: [sync](../spec/sync.md) ("An add accepted without a handle waits for its page", modified; "A delivered create waits for its arrival", added), [mutate](../spec/mutate.md) ("The offline mutation vocabulary": a `Remove` or `Move` of a delivered create), [upgrade](../spec/upgrade.md) ("A pending create is landed by its arrival").

## Open

- spec/REVISION still names pimdir 8c16bb1, so tests/spec_drift.rs reports the re-vendored files until pimdir's change is committed and the revision moved to it.
