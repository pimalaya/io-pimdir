---
cairn: tasks
change: duplicate-link-id-freeze
---

# Tasks

Landed 2026-08-25, superseded by `duplicate-link-id-mints-an-item` on 2026-08-28: the column went, the refusal stayed.

- [ ] ~~Bump io-replica to the release carrying `ambiguous_handles` and `ReplicaStatus::Ambiguous` (0.5).~~ Dropped: io-replica was taken as a path dependency instead, then folded into this crate by `engine-merge`.
- [x] Schema: `bindings.ambiguous_handles` (TEXT, JSON array, nullable), folded into version 1 and reconciled on open beside the other folded-in columns; update `sql::MIGRATION_0001` and the spec-fidelity fixtures. Removed again by the successor.
- [x] `write` persists the handles, `load` returns them on `ReplicaSourceBinding`, and both round-trip a store restart. Removed again by the successor.
- [x] `UPDATE_BINDING` no longer repoints an existing `(collection, link_id, source)` to a different handle: the bound handle stays, the incoming one is recorded as ambiguous. The refusal survives; the record does not.
- [x] Queue `add` of a link id already bound to a different handle takes the same path rather than parking or overwriting.
- [x] Tests: the repoint is refused and recorded; the handles survive a reopen; an ordinary write that names the bound handle clears nothing; a store written before the column reconciles on open. Rewritten by the successor as tests/duplicate_link_id.rs.
- [x] `pimdir check` counts ambiguous bindings in its report (read-only). Replaced by the minted-key count.
- [x] `cargo test`, `cargo clippy --all-targets`, `cargo fmt`; the spec-fidelity suite against the pimdir specification checked out beside the crate.
- [x] Prepare the release (breaking through the io-replica bump), CHANGELOG under `### Fixed` for the silent repoint and `### Added` for the column.
- [x] Fold `delta.md` into `cairn/spec/store.md`; add the `cairn/log` entry; mark the change `landed` and archive it.
- [ ] ~~Update the pimdir SPEC (§10 bindings, §13 columns) in the pimdir repo, the format being the contract this crate implements.~~ Dropped: the format settled on the successor's rule, a minted key for the second copy, and never carried the column.
