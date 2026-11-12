---
cairn: tasks
change: a-delivered-create-waits-for-its-arrival
---

# Tasks

- [x] spec/ re-vendored from pimdir's working tree; vectors 20, 92, 136, 137, 138 failing before the code change; 48 updated, 139 added (reworked by the review: waiting, reported); tests/vectors_sync.rs leaves an unscripted push unreported
- [x] `PimdirWriteOp::DeliverPlacement`, `PimdirHandle::is_provisional`, `PimdirPlacement::delivered`, `PimdirLinkId::is_landable`
- [x] sync: delivery on outcome, join narrowing, landing a delivered create, absences and counts
- [x] hub and client write fold the op in order; upgrade and rekey follow
- [x] unit tests in src/sync.rs and src/placement.rs; cairn/spec/sync.md, mutate.md, upgrade.md folded; CHANGELOG
- [x] Review 2026-11-12: `PimdirPushResult::relocated`, only a relocation delivering (vector 140, the identity property test with its mirror exemption narrowed to body-less delivered creates); sync-draft-09 (`SYNC_VERSION`, README, cairn/spec/store.md); sync vectors at format 4
- [x] spec/REVISION to pimdir's commit of the change once committed (tests/spec_drift.rs reports the re-vendored files until then)
