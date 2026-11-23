---
cairn: delta
change: an-io-free-indexer
---

Folds into `spec/search.md` and `spec/store.md`.

## ADDED Requirements

### Requirement: The index refreshes without I/O
The crate SHALL offer the index's refresh as a coroutine yielding the index state, the store's feed and the bodies it needs, and the writes it makes as typed operations, so an owner holding its own SQLite indexes without the crate opening a file.

#### Scenario: An owner in another language
- GIVEN a store owned by a Java program on its own SQLite
- WHEN it drives the refresh, answering each yield from its own databases
- THEN the index it writes answers every search vector's hit set

### Requirement: Search compiles without the client
The query language, extraction, tags, threads and occurrence expansion SHALL build without the `client` feature.

#### Scenario: A build without rusqlite
- GIVEN the crate built with `search` and without `client`
- WHEN a caller parses a query and extracts a message
- THEN both succeed, and no SQLite is linked

## MODIFIED Requirements

### Requirement: The index folds text before the tokenizer
The index SHALL store and match text folded by SEARCH §5's rule before any tokenizer reads it, and SHALL register no SQL function.

#### Scenario: A diacritic
- GIVEN a message whose subject is "Réunion"
- WHEN the query `reunion` runs
- THEN the message is a hit, whatever the shape's tokenizer

## REMOVED Requirements
