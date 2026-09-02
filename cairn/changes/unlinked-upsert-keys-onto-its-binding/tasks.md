---
cairn: tasks
change: unlinked-upsert-keys-onto-its-binding
---

# Tasks

- [x] Failing test first: a bound handle written again with no link id answers `load` with two placements.
- [x] `apply_ops` resolves an unlinked upsert through `LINK_FOR_HANDLE` before deciding between the hub and the residual.
- [x] The resolution is shared with the two sites that already ran the statement (`load`'s handle scope, `batch_links`'s drop arm) rather than copied a third time.
- [x] A handle no binding holds still stages unlinked, at the level and with the base the probe carried.
- [x] The rebind guard sees a resolved upsert: a batch claiming one identity under two handles is refused, and the refused batch leaves no residual row behind.
- [x] Neighbours held: `tests/duplicate_link_id.rs`, the hub and roundtrip suites, retention and revival, `write_rekeyed`.
- [x] `cargo test --all-features`, `cargo clippy --all-features --all-targets`, `cargo fmt`.
- [x] CHANGELOG `### Fixed` under `[Unreleased]`.
- [x] Fold `delta.md` into `cairn/spec/store.md`, append `cairn/log/2026-09-02-unlinked-upsert-keys-onto-its-binding.md`, mark this change `landed`.
