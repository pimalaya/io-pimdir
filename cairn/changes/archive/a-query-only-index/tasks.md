---
cairn: tasks
change: a-query-only-index
---

- [x] pimdir search-draft-05 landed (`occurring_between`, `has:attachment`, `horizon_left`, `id:`, `linked:`); spec/ re-vendored (tests/spec_drift.rs green)
- [x] Test: the query-only open of a missing index fails typed and creates nothing; of an outdated one, rebuilds nothing; its refresh is refused (tests/search.rs `a_query_open_neither_builds_rebuilds_nor_refreshes`; a new API, absent before)
- [x] src/client/search.rs: `PimdirIndex::open_query`, read-only connection; `IndexMissing`, `IndexOutdated`, `IndexQueryOnly`
- [x] `coverage.local.behind`, the store's stamps above the index cursor, in the library and the CLI's JSON and text
- [x] Test: `collection:` on a display name matches nothing (failed before: `[1]`); compile.rs, client/search.rs fixed
- [x] Test: a rename a cut pass folded past is reconciled at the next pass (failed before: nothing reconciled); `index_build.reconciled`, build 2. The cut is set up as a crash after a batch leaves it (its `store_change` written), the deadline being checked before a first batch.
- [x] Test: `id:` and `linked:` with digits name the seq and the link id (failed before: `id:777` named seq 777 alone); `PimdirQueryId::Digits`; `linked:x-:id` reads no role
- [x] Vectors: zero-length occurrence at range start, `has:attachment` field, `horizon_left`: every case of vectors/search passes (tests/search.rs)
- [x] pimdir gaps reported: the lag field (SEARCH §8.2), the reconciled stamp (SEARCH §4, `index_meta`), a vector for `id:` digits naming a link id
- [x] Undocumented extensions and own statements: moved to search-feature, which stays active
- [x] README marks `search` experimental; CHANGELOG
- [x] Spec: search.md folded; log 2026-10-30-a-query-only-index
