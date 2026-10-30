---
cairn: log
change: the-store-version-contract
date: 2026-10-30
---

# The store version contract

pimdir storage-draft-05 made the store version a contract (STORAGE §4.2, §6): refuse a store above the crate's version, never write it, reconcile only a version 1 store and before migrations, bump both stamps through a canonical statement, keep every row. Before it, io-pimdir ran migrations before the reconciliation, bumped `store_meta.version` with its own SQL, never read `store_meta.format`, set the journal mode before any check, reconciled before checking a declared algorithm, and its reader failed on a store 0.5.1, 0.7.0 or 0.8.0 wrote.

## What landed

- spec/: migrations/ and queries/ re-vendored byte for byte from pimdir (the closed `kind`, the open `role`, `level` 1 or 2, the pinned `shared_object`, `store_meta.derivation`, `collections_keep_queued`, and the statements `set_store_version`, `set_derivation`, `repair_shared_objects`, `repair_levels`, `repair_kinds`, `backfill_invitation`, `backfill_task_organisation`); tests/spec_drift.rs green.
- src/client/schema.rs: `init` runs §6 in order (refuse above, refuse `Foreign`, `Stale`, check the algorithm, set WAL, reconcile a version 1 store against 0001 alone with its repairs first and a recount last, then each migration with `set_store_version`), records `DERIVATION` through `init_store_meta` and `set_derivation`; `check` compares `store_meta.version` with the store's `user_version`; `check_version` lets a reader or producer read an older store; `summary_load_sql` reads the columns a later draft added as `NULL`.
- src/client.rs, src/client/producer.rs: `open_with_hash` and `open_current` refuse before writing; the producer sets WAL after its checks. New `PimdirError::Foreign` (`store-foreign`).
- tests/fixtures/store-v0.5.1, store-v0.7.0, store-v0.8.0 (0.8.0 from 3b9154a, which has no tag), written by tests/fixtures/store_gen.rs run in a `git archive` of each release.
- Tests: tests/store_version.rs, every one failing on 4c3353e (the reader: "no such column: invitation"; a newer store and a declared-algorithm mismatch: the header bytes changed; a foreign database: opened); src/client/schema.rs `migration::a_migration_runs_after_the_reconciliation_and_keeps_every_row` (a synthetic 0002 dropping an index the reconciliation recreates, so it fails if run before it).
- Capabilities moved: store ("Schema version", "The two schema stamps must agree", "Open reconciles against the canonical text", "An owner open that reconciles nothing", modified; "A store this crate may not write is never written", "A reader or a producer reads an older store as it stands", "A released store opens whole", "A migration runs after the reconciliation and keeps every row", "The store records the derivation revision", added).

## Deviation

The task "a store carrying a table or column this crate does not know keeps it through an owner open" was dropped: STORAGE §6 step 3 drops what 0001 lacks at version 1. A newer owner adds through a migration to version 2, which this crate refuses without writing.
