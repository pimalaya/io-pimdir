---
cairn: tasks
change: a-conflict-never-hides-a-server-edit
---

# Tasks

- [x] Wait for the pimdir side (SYNC §5, §7, vector 32 at `base_object: null`, new vector 33) in the sibling ../pimdir checkout.
- [x] src/sync.rs `reconcile_content`: an item-conflict placement (no `conflict_revision`) meeting a revision its base does not hold calls `mark_conflict` (see proposal). Unit test in the `mod tests` of src/sync.rs: placement `Conflict` with no revision, base `r1`, remote item `r2`; expect the upsert `conflict_revision: r2`, `conflict_object: None`, `report.conflicts == 1`, one `Conflicted` event.
- [x] src/mutate.rs `Remove`: the base adopts `conflict_object` with `conflict_revision`. Unit test in the `mod tests` of src/mutate.rs (beside `an_edit_resolves_a_divergence_a_tombstone_carries`): fetched diverging body becomes `base.object`; unfetched leaves `base.object == None`.
- [x] tests/engine/conflict_property.rs: pin the CI and the local sequences from the proposal as named `#[test]`s calling `check_conflict_model`, after `a_delete_elsewhere_does_not_swallow_a_local_divergence`, each with a doc comment saying which rule it covers.
- [x] Keep the CI seed line in tests/proptest-regressions/conflict_property.txt.
- [x] `cargo test --features cli` green, vectors_sync included (needs ../pimdir); `PROPTEST_CASES=1000 cargo test --release --features cli` green; clippy `-D warnings`; `cargo fmt`.
- [x] Fold the delta into cairn/spec/sync.md and cairn/spec/mutate.md; log cairn/log/2026-MM-DD-a-conflict-never-hides-a-server-edit.md; status `landed`.
- [x] CHANGELOG `[Unreleased]` `### Fixed`: a server edit made while an item was in a cross-source conflict was dropped silently; a remove settling a conflict left a base claiming the old body. Release prep only when asked (version bump, dated section).
- [ ] After the release: neverest picks it up (`cargo update -p io-pimdir`, tests).
