---
cairn: log
change: every-read-takes-an-older-store-as-reconciled
date: 2026-10-30
---

# Every read takes an older store as reconciled

The capability, queue, reference, attachment and file reads and the search index still ran canonical statements as they are over a store an older release wrote, or guarded them by "the table is missing, answer nothing", which is not what the reconciled store answers (STORAGE §6).

## What landed

- src/client/schema.rs: `StoreShape`, read when a reader or an index opens a store, runs a statement unchanged on a store holding every canonical table and column, else through `as_reconciled`. `nulled`, `missing_columns` and `TASK_ORGANISATION` removed.
- src/client/reader.rs: every statement through `reconciled_sql`, its `has_table` guards gone. src/client/capability.rs, src/client/producer.rs (`action_status`, the pending and duplicate reads), src/client/write.rs `kind_of`: through `as_reconciled`.
- src/client/search.rs, compile.rs, thread.rs, occurrence.rs: the compiled select and count, the feed, the reconciliation, the extraction, the thread and the occurrence reads naming the store go through the index's shape; the compile's private task-column rewrite is gone.
- `store-older` removed from `PimdirError::code()`, its rustdoc table and the spec: `Version` is raised above this crate's version alone (schema.rs `init`, `check_version`), the older-store message gone with it.
- Not routed, and why: owner-only paths (the drain, the write path, diagnostics, attachments' recording) read a store their open reconciled or `open_current` found current; the index's own tables were created at this build's shape, an index of another rebuilt.
- Tests: tests/store_version.rs `a_store_released_by_*_declares_as_its_reconciliation_would` (0.5.1 failed before: `capabilities` answered `[]` where the reconciled copy lists `imap` undeclared) and `a_store_released_by_*_is_indexed_and_searched` (0.5.1 and 0.7.0 failed before: "no such table: store.file_summary"). tests/files.rs `an_earlier_store_gains_the_files_on_open` now asserts the reconciled answer, the attachments without summaries, where it asserted none.
- Capabilities moved: store ("A reader or a producer reads an older store as it stands", "An error carries a stable code", modified), search ("The index reads an older store as its reconciliation would leave it", added).

## For the apps

A consumer matching `store-older` drops that arm. A reader of a store without capability tables now sees its syncing sources undeclared, which gates nothing, as before.
