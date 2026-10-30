---
cairn: log
change: the-index-reads-the-canonical-columns
date: 2026-10-30
---

# The index reads the canonical columns

pimdir search-draft-05 took four of this crate's private search mechanisms into the canonical schema and statements (pimdir log search-gaps-io-pimdir-found).

## What landed

- spec/: migrations/ and queries/ re-vendored byte for byte (`attached`, `behind`, `insert_object` binding `:attachment`, `set_index_cursor` binding `:store_reconciled`, `load_index_meta` answering it last); build.rs generates their constants unchanged; tests/spec_drift.rs green (it failed before: `ATTACHED` had no constant).
- src/client/search.rs: an index is current when its version, tokenizer and shape match migrations/search/0001_init.sql (tables, indexes and triggers by normalised `sqlite_schema` text, FTS5's shadow tables and other tables left out); `index_build`, `BUILD` and their statements removed; the refresh reads and writes `index_meta.store_reconciled`; `insert_object` records whether attachment text was extracted; the coverage's `behind` runs the canonical statement.
- src/client/search/compile.rs: `has:attachment` on a calendar item embeds `attached`; the `pimdir_column_size` function and the docsize decoding removed.
- Tests: tests/search.rs `an_index_of_another_shape_is_rebuilt_never_queried` (with the shape check removed: the index opened for queries); every search vector green.
- Capabilities moved: search ("The index is derived and rebuilt, never migrated", "A refresh folds the feed in bounded, resumable batches", "One-shot answers refresh first and say what is behind", "A calendar item has an attachment when its field is not empty", modified).

## For the apps

neverest's indexer rebuilds every index on its first open after this revision (the shape changed); himalaya's `open_query` answers `index-outdated` until then. No signature changed.
