---
cairn: log
change: what-pimdir-stated-for-the-store
date: 2026-10-30
---

# What pimdir stated for the store

pimdir storage-draft-05 landed three of this crate's store gaps (pimdir log store-gaps-io-pimdir-found).

## What landed

- src/client/schema.rs `as_reconciled`: a canonical statement read over a store an older release wrote, or one its owner has not reconciled yet, as its reconciliation would leave it (STORAGE §6): each canonical table the statement names after `FROM` or `JOIN` that the store lacks, or holds without some column, read through a subquery answering the canonical columns, a missing one as its `NOT NULL` default or `NULL`, a missing table as no row. On a current store every canonical statement runs unchanged (tested).
- src/client/reader.rs: the collection listings, `list_coverage`, `replied` and the task reads, and `schema::summary_load_sql` (the reader's and the index's summary loads), go through it; the guards rewriting statement text are gone. `list_coverage` on a store lacking the coverage columns lists each source with no coverage, where it listed none and so read every collection as complete.
- tests/scoped_sync.rs `an_earlier_draft_store_is_reconciled_on_open` now expects the source listed with no coverage, the rule STORAGE §6 states.
- Failing first: tests/store_version.rs `a_store_released_by_0_5_1_reads_as_its_reconciliation_would_leave_it` ("the source is listed": 0 rows); the 0.7.0 and 0.8.0 forms, and every other public read on all three fixtures, passed before.
- Checked, passing before: tests/unbound_items.rs `an_unbound_edit_over_a_replaced_body_parks` (`stage_action` checks `if_match` before the item is found unbound, so pimdir's reading of `apply_unbound` does not reproduce); tests/store_version.rs `the_reconciliation_drops_what_the_canonical_schema_lacks`. No statement of this crate creates schema in pimdir.db; the index's state is in index.db.
- Capabilities moved: store ("A reader or a producer reads an older store as it stands", "An action on an item no source binds is the store's to settle", modified; "pimdir.db holds the canonical schema alone", added).
