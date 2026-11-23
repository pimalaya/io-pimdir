---
cairn: delta
change: every-read-takes-an-older-store-as-reconciled
---

## ADDED Requirements

### Requirement: The index reads an older store as its reconciliation would leave it
Every statement the index runs naming the attached store goes through `schema::StoreShape` (STORAGE §6), so an older store is indexed and searched before its owner reconciles it.

## MODIFIED Requirements

### Requirement: A reader or a producer reads an older store as it stands
Every statement a reader, a producer or the search index runs over the store goes through `schema::as_reconciled` (through `StoreShape`, which runs it unchanged on a store holding every canonical table and column): the capability and performer reads, the queue and receipt reads, the reference, attachment, file and endpoint reads, and the index's compiled query, feed and extraction included. Owner-only paths read a store their open reconciled.

#### Scenario: Capabilities on a store without the capability tables
- GIVEN the 0.5.1 fixture, unreconciled, and a copy its owner reconciled
- WHEN a reader and a producer read capabilities, performers and action statuses
- THEN both answer alike, the syncing sources undeclared (tests/store_version.rs `a_store_released_by_*_declares_as_its_reconciliation_would`)

#### Scenario: Searching an older store
- GIVEN the 0.5.1, 0.7.0 or 0.8.0 fixture, unreconciled
- WHEN it is indexed and asked every field of SEARCH §8
- THEN every query answers, its task of no priority and no category, and pimdir.db is unwritten (tests/store_version.rs `a_store_released_by_*_is_indexed_and_searched`)

### Requirement: An error carries a stable code
`store-older` is removed: `Version` is raised above this crate's version alone, `store-newer`.
