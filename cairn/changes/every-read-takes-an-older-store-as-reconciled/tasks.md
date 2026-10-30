---
cairn: tasks
change: every-read-takes-an-older-store-as-reconciled
---

- [x] Test: a reader and a producer answer the capability, performer and action-status reads of each released fixture as they do once its owner reconciled a copy (tests/store_version.rs `a_store_released_by_*_declares_as_its_reconciliation_would`; 0.5.1 failed before: `Ok([])` where the reconciled copy lists `imap` undeclared)
- [x] Test: each released fixture, unreconciled, is indexed and answers every field of SEARCH §8 in every order, its task's missing columns read as none, the store unwritten (tests/store_version.rs `a_store_released_by_*_is_indexed_and_searched`; 0.5.1 and 0.7.0 failed before: "no such table: store.file_summary")
- [x] src/client/schema.rs: `StoreShape`; `nulled`, `missing_columns`, `TASK_ORGANISATION` removed
- [x] src/client/reader.rs: every statement through `reconciled_sql`; `has_table` guards removed
- [x] src/client/capability.rs, src/client/producer.rs, src/client/write.rs `kind_of`: through `as_reconciled`
- [x] src/client/search.rs, compile.rs, thread.rs, occurrence.rs: every statement naming the store through the index's shape
- [x] tests/files.rs `an_earlier_store_gains_the_files_on_open` asserts the reconciled answer (the attachments without summaries) rather than none
- [x] `store-older` removed: no site raises `Version` below this crate's version
- [x] CHANGELOG; spec store.md and search.md folded; log
