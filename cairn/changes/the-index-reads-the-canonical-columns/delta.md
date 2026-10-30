---
cairn: delta
change: the-index-reads-the-canonical-columns
---

## MODIFIED Requirements

### Requirement: The index is derived and rebuilt, never migrated
`PimdirIndex::open` SHALL open index.db as the main database with pimdir.db attached read-only as `store`, and SHALL rebuild it when its version, its tokenizer or its shape differs from the vendored schema's (SEARCH §2): every table, index and trigger migrations/search/0001_init.sql creates, compared by its `sqlite_schema` text with `--` comments removed and whitespace collapsed, the shadow tables FTS5 creates and any other table left out. An index an earlier draft or an earlier release of this crate built is therefore rebuilt rather than answering from data it lacks, and stored chunk offsets are always this crate's extraction's, which a snippet re-extracts (SEARCH §6). `PimdirIndex::open_query` SHALL refuse such an index as `PimdirError::IndexOutdated`. It SHALL refuse a SQLite without FTS5 or older than 3.43 (`PimdirError::Fts5`) rather than build a partial index.

#### Scenario: An index an earlier draft built
- GIVEN an index.db at version 1 whose `object` table lacks `attachment`
- WHEN `open_query` opens it, THEN it answers `IndexOutdated`; WHEN `open` opens it, THEN it rebuilds it and the next `open_query` opens it (tests/search.rs `an_index_of_another_shape_is_rebuilt_never_queried`)

### Requirement: A refresh folds the feed in bounded, resumable batches
`PimdirIndex::refresh` SHALL take index.lock without waiting, answering `busy` when another indexer holds it. It SHALL fold `load_changed_items` in transactions of a bounded number of rows, each full batch recording through `set_index_cursor` its last row's stamp with the purge count and `store_reconciled` the last completed pass recorded, so a pass cut short resumes where it stopped. The transaction exhausting the feed SHALL reconcile keys when the purge count moved or a collection was stamped above `index_meta.store_reconciled`, and record the cursor read before the pass as `store_change` and `store_reconciled`, with the purge count read beside it: no batch advances `store_reconciled`, so a rename a deadline-cut pass folded past is still reconciled, by this indexer or another one resuming the pass. A deadline SHALL be checked between batches. A handle opened by `open_query` SHALL refuse to refresh (`PimdirError::IndexQueryOnly`).

### Requirement: One-shot answers refresh first and say what is behind
`search` and `count` SHALL refresh up to the store's head within `--refresh-timeout` before answering (`0s` skips it), and SHALL count what stays unfolded as `coverage.local.unindexed`; every answer SHALL say in `coverage.local.behind` (`behind` in JSON) what the canonical `behind` answers: the change stamps the store drew above the index cursor, store-wide. A session that keeps the index open follows the feed instead.

### Requirement: A calendar item has an attachment when its field is not empty
The calendar extraction SHALL put each `ATTACH`'s `FILENAME` (or `X-FILENAME`) and `FMTTYPE` in the `attachment` field, `insert_object` SHALL record in `object.attachment` whether that text is non-empty, and `has:attachment` SHALL match a message whose `mail_summary.attachment` is 1 or a calendar item `attached` names (SEARCH §6, §8). No implementation function reads FTS5's shadow tables.

#### Scenario: The attachment vectors
- the search vectors `has:attachment` and `has:attachment kind:task` over items 13, 16 and 23 (tests/search.rs)
