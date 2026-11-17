---
cairn: delta
change: a-reader-answers-from-one-snapshot
---

## ADDED Requirements

### Requirement: A reader answers from one snapshot
A reader answer spanning several statements (`list_items_page_*` and every listing through the overlay, `count_items`) SHALL read them in one read transaction (STORAGE §8), so a sync committing between two of them is seen whole or not at all, rolled back when the answer is read; a connection in a transaction already, an owner reading through its own, reads in that one.

### Requirement: The index sets its own journal mode alone
The indexer SHALL set the journal mode of index.db alone (`PRAGMA main.journal_mode`), never that of the store it attaches read-only, so a store its operator keeps on a rollback journal (STORAGE §8, a network filesystem) is indexed and searched as any other.

#### Scenario: A rollback-journal store
- GIVEN a store switched to `journal_mode = DELETE`
- WHEN it is indexed and searched, by the indexer and by a query client
- THEN both answer (tests/search.rs `a_store_on_a_rollback_journal_is_indexed`; before, `attempt to write a readonly database` and a 0-byte index.db)

### Requirement: An exported body is the store's user's alone
`item export --output` SHALL create its file `0600` (STORAGE §18), as `export` creates its dump: it holds what the store holds.

## MODIFIED Requirements

## REMOVED Requirements
