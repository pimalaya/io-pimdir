---
cairn: change
id: a-delivered-create-waits-for-its-arrival
status: landed
created: 2026-11-11
---

# A delivered create waits for its arrival

## Why

Follows pimdir's change of the same id (pimdir cairn/changes/a-delivered-create-waits-for-its-arrival). A create its source took before any listing named it, an add accepted with no handle or a relocation into it accepted, was left a pending create: a `Remove` or a `Move` staged on it withdrew it (mutate.rs, a tombstone with no base dropped), nothing reached the server and the next listing brought the message back. The Android app hit it on Gmail (android cairn/log/2026-11-11-gmail-trash-stays.md).

## What

- `PimdirWriteOp::DeliverPlacement { collection, handle }`, written by the sync for an add accepted with no handle and, into the destination's collection, for an accepted relocation, when a listing can land the create (`PimdirLinkId::is_landable`). The client folds it into the hub in order; the hub bases a binding under a provisional handle only while it holds none.
- `PimdirHandle::is_provisional`, `PimdirPlacement::delivered`.
- `PimdirPushResult::relocated`: only an accepted `Remove` the connector states it relocated delivers the destination's create.
- sync-draft-09: `SYNC_VERSION` moves, so a store an engine of sync-draft-08 wrote is refused.
- The sync: a delivered create is no candidate of the join's narrowing while unlisted, never found absent nor counted; `take_pending_create` lands it; `land` keeps a tombstone's status and destination. It waits whatever a round lists, reported in `PimdirSyncReport::waiting` (review 2026-11-11, replacing a re-pend on a closing round).
- The upgrade fetches no provisional handle; the rekey carries a delivered create onto the member resolving to it and otherwise leaves it untouched and uncounted.
- spec/ re-vendored from pimdir's working tree.
