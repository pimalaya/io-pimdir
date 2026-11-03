---
cairn: log
change: the-crate-follows-the-freeze-gaps
date: 2026-11-03
---

# The crate follows the freeze gaps

Follows pimdir cairn/log/2026-11-03-the-freeze-gaps-fold-into-draft-08.md, re-vendored at pimdir 6687cf4 (spec/REVISION). Vector 131 passed before the change; 132 to 135 failed and pass now.

- **An edit over an unread base** (src/placement.rs `awaits_base`, src/sync.rs, src/upgrade.rs; sync.md, upgrade.md). The content axis derives no `Update` over a base naming a revision and no body, nor the ridden `Remove`; `Full` revisits such a placement. tests/engine/property.rs's `hydrate` now also upgrades an awaiting base at `Full`, as a consumer must: without it two convergence properties ended with the edit `Dirty`.
- **A capped listing** (src/remote.rs, src/sync.rs; seam.md, sync.md). `PimdirRemoteSnapshot::capped`, read on a round's last page: its absences are counted in `PimdirSyncReport::unlisted` and dropped nowhere.
- **A bodiless copy** (src/mutate.rs, src/client/run.rs; mutate.md). `PimdirMutate::syncs_target`, `false` by default, set by `PimdirSourceStore::mutate` from `collection_sources`; the four unit tests of a same-source copy state it.
- **Runner** (tests/vectors_sync.rs). Reads `capped`, and a Copy's and a Move's target from `to`, as the schema names it: no vector had run a Copy before 135.

Not done here: `Overrule` and `Settle` were already the crate's; pimdir STORAGE §14 now names them.
