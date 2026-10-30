---
cairn: tasks
change: the-store-version-contract
---

- [x] pimdir storage-draft-05 landed (spec edit 1); spec/ re-vendored, tests/spec_drift.rs green
- [x] Fixtures: stores written by v0.5.1, v0.7.0 and 0.8.0 (3b9154a, untagged), with pending, parked and applied actions, retained items and, from 0.8.0, a reference, checked in under tests/fixtures/
- [x] Test: each fixture opens as owner and reads back every row (failed on 4c3353e: the reader failed "no such column: invitation")
- [x] Dropped, against the spec: "a store carrying a table or column this crate does not know keeps it through an owner open"; STORAGE §6 step 3 drops what 0001 lacks at version 1; a newer owner's additions are a migration to version 2, which this crate refuses (tested)
- [x] Test: a foreign database is refused and left as it was (failed on 4c3353e: opened)
- [x] src/client/schema.rs: format check, refuse above, reconcile at version 1 against 0001 alone before migrations, repairs first, canonical bump and backfills
- [x] src/client.rs: `open_with_hash` checks the algorithm before writing (failed on 4c3353e: the header changed)
- [x] Synthetic 0002 test: runner, both stamps, every table kept
- [x] Reader and producer rule for an older store
- [x] `store_meta` derivation revision (Annex A), as the spec states it
- [x] CLI owner verbs reviewed against the contract: all go through `open_current`, which refuses `Foreign` too
- [x] Spec: store.md ("Schema version", "Open reconciles against the canonical text"); fold, log, CHANGELOG
