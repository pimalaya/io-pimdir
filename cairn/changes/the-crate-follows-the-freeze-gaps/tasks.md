---
cairn: tasks
change: the-crate-follows-the-freeze-gaps
---

- [x] spec/ re-vendored from pimdir; vectors 131 to 135 run, 132 to 135 failing before the change
- [x] src/sync.rs, src/placement.rs `awaits_base`: no `Update` and no ridden `Remove` over an unread base
- [x] src/upgrade.rs: `Full` revisits a placement awaiting its base
- [x] src/remote.rs, src/sync.rs: `capped`, `PimdirSyncReport::unlisted`
- [x] src/mutate.rs, src/client/run.rs: `syncs_target`; the four same-source copy tests state it
- [x] tests/vectors_sync.rs: `capped`, a Copy's `to`; tests/engine/property.rs hydrates an awaiting base at `Full`, as a consumer does
- [x] Spec folded (mutate.md, seam.md, sync.md, upgrade.md), log, CHANGELOG
- [x] spec/REVISION at pimdir 6687cf4
