---
cairn: log
change: a-query-only-index
date: 2026-10-30
---

# A query-only index

The plan of 2026-10-30 makes the store's owner the indexer and frontends (himalaya's `pimdir search`) query clients only, which SEARCH §3 already asks: a query client opens index.db read-only. `PimdirIndex::open` created, rebuilt and refreshed, read-write with WAL.

## What landed

- src/client/search.rs: `PimdirIndex::open_query(dir)`, index.db read-only, no lock, no journal-mode write, never built, rebuilt nor refreshed; `refresh` on it is `PimdirError::IndexQueryOnly`. New `PimdirError::IndexMissing(PathBuf)` (`index-missing`), `IndexOutdated(PathBuf)` (`index-outdated`), `IndexQueryOnly(PathBuf)` (`index-query-only`), each saying to run the store's sync engine.
- src/search/coverage.rs: `PimdirSearchLocal::behind`, the store's change stamps above the index cursor, store-wide, in every answer; the CLI's coverage carries it (`behind` in JSON, a line in text).
- `collection:` and `folder:` match collection ids only, in the compiled query and the coverage's scope (src/client/search/compile.rs, src/client/search.rs).
- A rename a deadline-cut pass folded past is reconciled by the next completed pass: src/sql/search.rs `index_build.reconciled`, the store's last stamp when the last completed pass began, `LOAD_RECONCILED`, `SET_RECONCILED`; `BUILD` 2, so an older index is rebuilt.
- `id:` and `linked:` with digits name the `seq` and every item of that link id (SEARCH §8): `PimdirQueryId::Digits(String)` replaces `Seq(i64)`, with `seq()` and `as_str()`; `LOAD_ENDS` matches either; `linked:x-:id` reads no role.
- README and cairn/spec/search.md mark `search` experimental and claim search-draft-05.
- Tests: tests/search.rs `a_query_open_neither_builds_rebuilds_nor_refreshes`, `a_collection_term_names_an_id_never_a_display_name` (failed before: `[1]`), `a_rename_a_cut_pass_went_past_is_reconciled_by_the_next_one` (failed before: not reconciled), `an_id_of_digits_names_the_seq_and_the_link_id` (failed before: `id:777` named nothing); src/search/query.rs `a_linked_value_reads_a_role_only_when_it_names_one`. Every vectors/search case passes, `occurring_between`, `has:attachment` and `horizon_left` included.
- Capabilities moved: search ("The index is derived and rebuilt, never migrated", "A refresh folds the feed in bounded, resumable batches", "One-shot answers refresh first and say what is behind", modified; "A query client opens the index read-only and builds nothing", "A collection term names an id", "A value of digits names a seq and a link id", "A rename past a cut pass is reconciled", added).

## Gaps reported to pimdir

- SEARCH §8.2 states no lag of the index behind the store: `behind`, the stamps above `index_meta.store_change`, is this crate's proposal.
- SEARCH §4 compares a collection stamp with the cursor, which a batch advances, so a cut pass loses a rename; the index needs the stamp the last completed pass began at, an `index_meta` column and so an index schema major (this crate keeps it in its own `index_build`).
- No vector names a link id of digits with `id:`.

## Migration

himalaya opens the index with `open_query` and maps `index-missing` and `index-outdated` to its "run the sync" message; neverest keeps `open` and `refresh`. A consumer matching `PimdirQueryId::Seq(n)` matches `Digits(text)` and reads `seq()`.
