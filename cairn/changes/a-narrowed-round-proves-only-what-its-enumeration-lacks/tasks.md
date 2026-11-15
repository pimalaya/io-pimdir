---
cairn: tasks
change: a-narrowed-round-proves-only-what-its-enumeration-lacks
---

# Tasks

- [x] spec/ re-vendored from the pimdir working tree (migrations/, queries/, vectors/, payloads/ copied whole)
- [x] Vectors 107, 160 and 161 fail with the old statement baked in, pass with the new (2026-11-15)
- [x] src/sync.rs: the last page loads unbound, the proof applied at the merge, `stamped` reads every stamp, `State::Opening` gone; src/load.rs, src/remote.rs docs
- [x] tests/scoped_sync.rs `a_resumed_narrowed_round_handing_no_enumeration_drops_nothing`, failing on the old engine and statement (it parked, 1 of 3 absent)
- [x] cairn/spec/sync.md "A narrowed listing proves only what its enumeration lacks", "Absence means deleted in scope only", "A capped listing proves no absence"; seam.md "A narrowed listing says so"
- [x] `cargo fmt --check`, `cargo clippy --all-features --all-targets -D warnings`, the suite in CI's three configurations, tests/spec_drift.rs alone failing on the uncommitted spec (2026-11-15)
- [x] spec/REVISION to the pimdir commit carrying the change (pimdir 6ccc3d4); tests/spec_drift.rs passes
- [x] The owner's decision on pimdir's reading: (a), taken on 2026-11-15 and landed in draft-12 (cairn/log/2026-11-15-the-crate-follows-draft-12.md)
