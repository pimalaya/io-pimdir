---
cairn: change
id: every-read-takes-an-older-store-as-reconciled
status: landed
created: 2026-10-30
---

# Every read takes an older store as reconciled

## Why

the-store-version-contract routed the collection listings, `list_coverage`, `replied`, the task reads and the summary loads through `schema::as_reconciled` (STORAGE §6's reader rule), but other reads still bypassed it:

- the capability reads (`capabilities`, `item_capabilities`, `accepts_vtodo`, `performers`, the gate) answered nothing when the `capabilities` table was missing, where the reconciled store lists each syncing source undeclared;
- the reference, attachment, file and endpoint reads answered no row when `item_reference` or `file_summary` was missing, where the reconciled store answers the rows the other tables hold;
- the producer's queue and receipt reads, the search's compiled query (its task columns through a private `nulled` rewrite, `store.file_summary` unguarded, so every search on a 0.5.1 or 0.7.0 store failed with "no such table: store.file_summary"), its feed and its extraction ran the canonical text as it is.

Readers and producers now open older stores and only the owner migrates them, so `PimdirError::Version` is raised only above this crate's version (src/client/schema.rs `init` and `check_version`): the `store-older` code could no longer be answered.

## What

- `schema::StoreShape`: whether a store holds every canonical table and column, read once when a reader or an index opens it; `sql(statement)` answers the statement itself on a whole store, else `as_reconciled`.
- The reader routes every statement through it, its `has_table` guards removed; the capability reads, the producer's `action_status`, pending and duplicate reads and `write::kind_of` through `as_reconciled`; the index its compiled select and count, its feed, reconciliation, extraction, thread and occurrence reads that name the store. `schema::nulled`, `missing_columns` and `TASK_ORGANISATION` are gone.
- Owner-only paths (the drain, the write path, diagnostics) are not routed: the owner open reconciles before any statement runs and `open_current` refuses a store that is not current, so the store they read is whole. The index's own tables are not routed: the indexer created them at this build's shape, rebuilding any other.
- `store-older` removed from `PimdirError::code()`, its rustdoc table and the spec; `Version` reads "newer" alone.
