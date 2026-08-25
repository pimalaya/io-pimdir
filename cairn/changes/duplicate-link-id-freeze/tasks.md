---
cairn: tasks
change: duplicate-link-id-freeze
---

# Tasks

- [ ] Bump io-replica to the release carrying `ambiguous_handles` and
      `ReplicaStatus::Ambiguous` (0.5).
- [ ] Schema: `bindings.ambiguous_handles` (TEXT, JSON array, nullable), folded
      into version 1 and reconciled on open beside the other folded-in columns;
      update `sql::MIGRATION_0001` and the spec-fidelity fixtures.
- [ ] `write` persists the handles, `load` returns them on
      `ReplicaSourceBinding`, and both round-trip a store restart.
- [ ] `UPDATE_BINDING` no longer repoints an existing
      `(collection, link_id, source)` to a different handle: the bound handle
      stays, the incoming one is recorded as ambiguous.
- [ ] Queue `add` of a link id already bound to a different handle takes the
      same path rather than parking or overwriting.
- [ ] Tests: the repoint is refused and recorded; the handles survive a reopen;
      an ordinary write that names the bound handle clears nothing; a store
      written before the column reconciles on open.
- [ ] `pimdir check` counts ambiguous bindings in its report (read-only).
- [ ] `cargo test`, `cargo clippy --all-targets`, `cargo fmt`; the spec-fidelity
      suite against the pimdir specification checked out beside the crate.
- [ ] Prepare the release (breaking through the io-replica bump), CHANGELOG
      under `### Fixed` for the silent repoint and `### Added` for the column.
- [ ] Fold `delta.md` into `cairn/spec/store.md`; add the `cairn/log` entry;
      mark the change `landed` and archive it.
- [ ] Update the pimdir SPEC (§10 bindings, §13 columns) in the pimdir repo, the
      format being the contract this crate implements.
