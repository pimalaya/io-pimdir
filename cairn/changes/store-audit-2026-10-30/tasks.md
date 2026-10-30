---
cairn: tasks
change: store-audit-2026-10-30
---

- [x] S1: the blob temporary name is shared between writers (src/client/blobs.rs:165, against STORAGE.md:154): a per-writer name in the shard directory, landed with the-store-follows-storage-draft-05
- [x] S4: SQL outside queries/ (src/client/schema.rs:76, :281, :323; string surgery on canonical text at src/client/reader.rs:575-590): `set_store_version`, `backfill_invitation`, `backfill_task_organisation` landed with the-store-version-contract; the reader's NULL substitution stays, no canonical statement reading a store that lacks a column (STORAGE §6 asks it to run no statement naming one)
- [x] S5: WAL hardcoded (src/client.rs, src/client/producer.rs): the journal mode is set at creation alone, so a store switched to a rollback journal for a network filesystem stays on it; README states the switch. tests/store_rules.rs `a_store_on_a_rollback_journal_stays_on_it` (failed before: the producer set WAL again)
- [x] S6: Annex A deviations (lossy non-UTF-8 charsets, MIME depth 64) stated in README beside cairn/spec/summaries.md; pimdir Annex A stating the bounds is pimdir's call (reported)
- [x] N8: a producer holds `objects.lock` shared for its staging window alone. tests/staging.rs `an_idle_producer_keeps_no_collector_out` (failed before: the collector was refused beside an idle producer); tests/queue.rs `gc_never_sweeps_a_queued_body` no longer expects a refusal after the enqueue committed
- [x] N9: `purge_retained_before` refuses a cutoff not written as the stamps are (`PimdirError::Cutoff`, `cutoff-invalid`), the signature kept (`&str`) for its callers. tests/retention.rs `a_purge_cutoff_not_written_as_the_stamps_are_is_refused` (failed before: `now` purged the trash)
- [x] N10: closed as unproven: no failing test, the suite's SQLite being 3.4x; a system SQLite below 3.37 already fails the first statement over a `STRICT` table, so the cost is an untyped message, not data
- [x] N14: closed as unproven: `put_item` and `collect_garbage` both take the owner handle `&mut`, the owner lock is one handle per process and `Connection` is `!Sync`, so no collector of this process runs beside it
- [x] Spec: store.md folded; log 2026-10-30-store-audit-2026-10-30; CHANGELOG
