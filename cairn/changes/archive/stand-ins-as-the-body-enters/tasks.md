---
cairn: tasks
change: stand-ins-as-the-body-enters
---

# Tasks

- [x] The streaming walk, its chunking, depth and count bounds (unit tests in src/summary/mail/attachments.rs).
- [x] The hook in `apply`, read before the diff, recorded after it, in the batch's transaction.
- [x] `put_file` and the hook share `insert_file` and `upsert_file_summary`.
- [x] tests/attachments.rs: two stand-ins with their names, types, sizes and parts; nothing written again for the same body, a second label, a release and a re-fetch; nothing backfilled; none for a derived key, inline parts or another kind; a drained `add`; a store with no account.
- [x] tests/references.rs: a rule links at the write and never recomputes.
- [x] summaries.json's `attachments` in tests/summaries.rs, the walk fed whole and in chunks; sync vector 70 in tests/vectors_sync.rs, which now reads `references` and a collection's `name` and `role`.
