---
cairn: log
change: five-freeze-decisions
landed: 2026-11-25
---

# Five freeze decisions

Follows the owner's decisions of 2026-11-25 as pimdir lands them (pimdir cairn/log/2026-11-25-five-freeze-decisions.md): storage-draft-18, sync-draft-16 and search-draft-18.

## What landed

- src/mutate.rs: `PimdirMutate::target_synced` (`true` by default) and `PimdirMutateError::Unsynced` (`target-unsynced`): a `Copy` or a `Move` into a collection no source syncs is refused before it reads its target, staging nothing. `staged_copy` carries the copied placement as origin only where the run's source syncs the target (`syncs_target`). `PimdirMutation::create_target` is public, for a runner of its own.
- src/client/write.rs: `Staging` (the target and the source its create is staged under, `Staging::of`), `syncing` (the source an `add` into a collection runs as: the owner's own where it syncs it, else the first), `apply_staged` (the batch's upserts into the target applied as that source, the rest as the run's, in the caller's transaction), `PimdirSourceStore::write_staged` and `load_as` (`load_in` takes the source it projects for).
- src/client/run.rs: `PimdirSourceStore::mutate` runs a copy or a move whose source does not sync its target through its own loop: the target's loads as the staging source, the write through `write_staged`.
- src/client/producer.rs: the drain's `stage_action` takes the drain's own source and answers the staging beside the writes; its target reads project for the staging source; `apply_claimed` applies through `apply_staged`; `acting_source` shares `syncing`.
- src/client/reader.rs: the overlay folds a pending `move` into a collection no source syncs as nothing and shows no arrival there, as the drain parks it.
- The versions: `sql::FORMAT_VERSION` storage-draft-18, `sql::search::FORMAT_VERSION` search-draft-18 and `FORMAT_DRAFT` 18, `sync::SYNC_VERSION` sync-draft-16, spec/queries/storage/owner/init_store_meta.sql and spec/queries/search/init_index_meta.sql, README.md, and the version tests (src/version.rs, tests/store_version.rs, tests/open_current.rs, tests/cli_owner.rs, tests/search.rs) moved to N-1 and N+1.

## Guards

- vectors/sync/227, 228 and 230 (the create bound to the source syncing the target, that source's own tombstone revived) and 229 (refused): all four fail on a `git archive` snapshot of 769dfba, by `store.mutate` and, for 227 to 229, by the drain (`bindings rows differ`, the create bound to `a`; 229 `the run failing differs`), and pass now. 224 to 226 (keep-local over flags and absence, a stated `alt:` key) pass on both: io-pimdir already read them so.
- tests/drain_copy.rs `a_drained_copy_revives_the_tombstone_of_the_source_syncing_its_target` (vector 230 through the drain, which the vector drain skips, two sources binding there) and `a_drained_copy_stages_its_create_under_the_owners_own_source` (the drain's own source chosen among two syncing the target): both fail on the snapshot.
- src/mutate.rs `a_copy_another_source_stages_carries_no_origin`, `a_copy_or_a_move_into_a_target_no_source_syncs_is_refused`; tests/overlay.rs `a_move_into_a_collection_no_source_syncs_folds_into_nothing`.
- tests/json_contract.rs `every_schema_id_names_the_contract_version`: every schema's `$id` names `v0.<minor>` below 1.0 and the major from 1.0, which `formatVersion` states (the_version_probe_states_the_crate_major). The code already followed the rule through pimalaya-cli's `JsonSchemaRegistry`; nothing pinned it here.
- The attachments collection id, a MUST now: vector 70 pins `\u{1}attachments`, tests/attachments.rs `work/\u{1}attachments`.

Tests moved to the rule rather than the rule to the tests: tests/engine/integration.rs `offline_copy_creates_pushes_and_rekeys`, tests/engine/membership.rs (three cases), tests/overlay.rs (four), tests/queue.rs `copy_fills_the_target_and_move_also_empties_the_source`, tests/receipts.rs `every_receipt_names_the_item_its_row_acted_on`, tests/item_conflict_settle.rs `a_move_over_a_conflicted_item_is_refused_and_parks` and tests/invariants_queue.rs `a_parked_move_retried_after_a_rename_lands_in_the_renamed_target` copied or moved into a collection no source had synced; their fixtures now sync it first (an empty listing, or a checkpoint), as the engine's property tests do at seeding (tests/engine/property.rs, policy_property.rs, hostile_property.rs), so the generated copies keep reaching the staging path instead of being refused.

## Spec

[mutate](../spec/mutate.md): new *A copy or a move stages its create under a source syncing the target*, the tombstone revive read as the staging source's, the `Add`'s `alt:` key. [sync](../spec/sync.md): `KeepLocal` governs content alone. [store](../spec/store.md): the attachments id a MUST, the draft names. [search](../spec/search.md): search-draft-18. [cli](../spec/cli.md): the `$id` scenario at `v0.20` with its test.

spec/REVISION still names bd36f0e: tests/spec_drift.rs `the_vendored_copy_is_the_spec_byte_for_byte` fails on the vendored files until the pimdir commit exists and REVISION names it.

## Left

- The Android app drives `PimdirMutate` through its own storage host (android/rust/src/offline.rs): it loads and writes a copy's or a move's target as the staging source and passes `target_synced`, or keeps staging under the run's source.
- A person filing an item into a collection no source syncs writes it there (`put_item`, `put_file`); no queue action does it for a message.
