---
cairn: delta
change: what-pimdir-stated-for-the-store
---

## MODIFIED Requirements

### Requirement: A reader or a producer reads an older store as it stands
A reader or a producer meeting a store below this crate's version, or a version 1 store its owner has not reconciled yet, SHALL read it as its reconciliation would leave it before any backfill (STORAGE §6): a column it lacks as its canonical `DEFAULT` where 0001_init.sql declares it `NOT NULL` with one, else `NULL`, a table it lacks as empty, running no statement naming either. `schema::as_reconciled` runs a canonical statement with each table the store lacks, or holds without some column, read through a subquery answering the canonical columns, the statement's parameters and result columns kept; the collection listings, `list_coverage`, `replied`, the task reads and every summary load go through it, and the reference, attachment and file reads answer no row when their table is missing. A source whose store lacks the coverage columns therefore never closed a round: `list_coverage` lists it with no coverage, and a search over its collection is filling, never complete.

#### Scenario: The released fixtures, read as they stand
- GIVEN a store io-pimdir 0.5.1, 0.7.0 or 0.8.0 wrote, with a source syncing INBOX
- WHEN a reader and a producer read it
- THEN `list_coverage("INBOX")` lists the source with no coverage and no round, `search_coverage` is `Bounded { filling: true }`, every public read answers, and pimdir.db is byte for byte as it was (tests/store_version.rs `a_store_released_by_*_reads_as_its_reconciliation_would_leave_it`; 0.5.1 failed before the change: no source listed)
- src/client/schema.rs `a_statement_reads_an_older_store_as_its_reconciliation_leaves_it` (a `NOT NULL` default, `categories` as `'[]'`, a missing table as no row), `a_current_store_runs_the_statement_unchanged`

### Requirement: An action on an item no source binds is the store's to settle
The drain SHALL never skip an action on an existing item no source binds, which would leave the row pending for ever (STORAGE §15.2 reserves skipping for a kind, a payload version or an intent the owner does not run). A `remove` SHALL retain the item, a delete nobody pushes. In a collection no source syncs, an item kept on the device (`put_item`, STORAGE §14.3), a `set-flags` SHALL set its flags and an `update` SHALL write its new body as `put_item` writes one, its flags kept and its summary derived again, on an `update`'s usual terms: one whose `if_match` names another body than the item's parks naming both (STORAGE §15.3), checked before the item is found unbound; a body deriving no summary, or another key than the item's, parks. A `move` or a `copy` of such an item, and anything on an unbound item of a synced collection, SHALL park naming the item. Each applied row leaves its receipt and releases its pin as any other.

#### Scenario: A device-only event edited, flagged, moved and removed
- tests/unbound_items.rs `a_queued_action_on_an_item_no_source_binds_applies_or_parks` (failed before the change: the drain reported both rows skipped)

#### Scenario: A device-only event edited over a replaced body
- tests/unbound_items.rs `an_unbound_edit_over_a_replaced_body_parks` (passed before this change: the fence was already checked in `stage_action`)

## ADDED Requirements

### Requirement: pimdir.db holds the canonical schema alone
The owner's reconciliation of a version 1 store SHALL drop every table, column, index and trigger 0001_init.sql lacks, whoever added it (STORAGE §6), and this crate SHALL keep no state of its own in pimdir.db: the search index's lives in index.db (SEARCH §2), the locks in their own files.

#### Scenario: Another implementation's schema
- GIVEN a current store to which another implementation added a table, a column, an index and a trigger
- WHEN its owner opens it
- THEN all four are gone, every canonical row is kept, and `open_current` finds it current (tests/store_version.rs `the_reconciliation_drops_what_the_canonical_schema_lacks`)
