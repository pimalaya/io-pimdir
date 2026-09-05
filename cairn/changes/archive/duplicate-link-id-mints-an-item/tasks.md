---
cairn: tasks
change: duplicate-link-id-mints-an-item
---

# Tasks

- [x] Bump io-replica to the release that mints the key and drops `ambiguous_handles` / `ReplicaStatus::Ambiguous` from `ReplicaSourceBinding` and `ReplicaPlacement`. *(Taken through `[patch.crates-io] io-replica.path = "../io-replica"` while nothing is published, so the whole local tree tests end to end. The patch goes back to git or crates.io when io-replica 0.5 is released.)*
- [x] Schema: `bindings.ambiguous_handles` removed from the canonical mirror and from `sql::MIGRATION_0001`; the open-time reconciliation drops it from an existing table, inside the transaction that already reconciles columns and indexes. *(`ALTER TABLE … DROP COLUMN` rather than the table rebuild SPEC §6 prescribes for a constraint: no index, key or check names the column, and rebuilding would mean a second copy of the canonical `bindings` DDL to drift from.)*
- [x] `write` stops writing the column, `load` stops selecting it, `codec::handles_to_json` / `handles_from_json` go.
- [x] `UPDATE_BINDING`: a handle change on an existing `(collection, link_id, source)` is refused with a typed `PimdirError`, per handle, with `ReplicaDropReason::Superseded` keeping its licence.
- [x] Queue `add`: unchanged, it still parks on a duplicate `link_id` (pimdir SPEC §15.3). Minting is for what a source hands over; a locally authored item colliding with a stored one is the producer's error and is parked, not minted.
- [x] Any other write reaching `bindings` outside the engine takes the refusal above, since not every write passes through the engine's identity resolution.
- [x] A minted key round-trips untouched through `write`, `load`, `seq_for_link_any`, retention, revival and the reader pages; no code path parses a prefix.
- [x] CLI: `pimdir item` drops `ambiguous_handles` from its JSON shape (breaking for consumers); `pimdir check` drops the ambiguity counter and MAY count minted keys per collection.
- [x] Tests: the colliding write is refused and stores nothing; a superseded drop still rebinds; two items under one hint each keep their `seq` and their object reference; a store written with the column reconciles on open; a byte-identical pair shares one object with refcount two.
- [x] Spec-fidelity suite green against a pimdir checkout carrying the SPEC change and the new vectors.
- [x] `cargo test`, `cargo clippy --all-targets`, `cargo fmt`.
- [x] Release prep: breaking through the io-replica bump; CHANGELOG `### Fixed` for the identity that stored only one of two copies. *(No `### Removed`: neither the column nor the CLI field was ever released, both landing inside this same unreleased window, so the net diff drops their `### Added` bullet instead of announcing a removal.)*
- [x] Fold `delta.md` into `cairn/spec/store.md`, append `cairn/log/YYYY-MM-DD-duplicate-link-id-mints-an-item.md`, mark this change `landed` and archive it beside `duplicate-link-id-freeze`.
