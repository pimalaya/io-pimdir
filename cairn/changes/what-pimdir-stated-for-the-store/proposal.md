---
cairn: change
id: what-pimdir-stated-for-the-store
status: landed
created: 2026-10-30
---

# What pimdir stated for the store

## Why

pimdir landed three of this crate's store gaps in storage-draft-05 (its log store-gaps-io-pimdir-found, 2026-10-30), and names where this crate falls short:

1. STORAGE §15.3: an `update` on an item no source binds, in a collection no source syncs, applies "on its usual terms, `if_match` included". pimdir reads `apply_unbound` as applying without the check.
2. STORAGE §6, the reader of an older store: a missing column reads as `NULL`, or as its canonical `DEFAULT` where 0001_init.sql declares it `NOT NULL` with one; a missing table reads as empty; a canonical statement runs with each mention of a missing column replaced by that value, its name, parameters and result columns kept. `list_coverage` answers no row on a store lacking the coverage columns, which reads every collection as complete where the rule reads its sources as filling; `task_summary.categories` reads `NULL` where its default is `'[]'`; and the guards are spread over call sites, each its own reading of the rule, some statements unguarded.
3. STORAGE §6, pimdir.db holds the canonical schema alone: the reconciliation drops every table, column, index and trigger 0001_init.sql lacks, and no implementation keeps state of its own in pimdir.db.

## What

1. A test that an unbound `update` whose `if_match` names another body parks, the item unchanged; the check moves into `apply_unbound` should the test fail.
2. One adapter, `schema::as_reconciled`, for the statements a reader runs over what an older store lacks: each table 0001_init.sql declares that the store lacks, or holds without some column, read through a subquery answering the canonical columns, a missing one as its `NOT NULL` default or `NULL`, a missing table as no row, the statement's parameters and result columns kept. It replaces the ad hoc guards that rewrote a statement's text (`collections_sql`, `task_sql`, the `replied` and `list_coverage` guards, `summary_load_sql`); the reads whose whole table is missing keep answering no row before running anything. Tests on the v0.5.1, v0.7.0 and v0.8.0 fixtures: a source with no coverage columns reads as filling (never complete), the search coverage says `filling`, every public read of a reader and a producer answers, nothing is written.
3. Check the reconciliation drops exactly what 0001 lacks (another implementation's table, column, index and trigger), and that no state of this crate's lives in pimdir.db (the search index's lives in index.db).
