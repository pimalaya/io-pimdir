---
cairn: change
id: item-sort-key
status: landed
created: 2026-08-08
---

# Order a collection by its kind's own key

## Why

`list_items_page` is `ORDER BY link_id`, and there is nothing else to order by:
an item carries `collection`, `link_id`, `seq`, `flags`, `object_hash`, `meta`,
`level` and the conflict and retention columns, and a date lives *inside* the
opaque `meta` blob. So a client cannot ask this store for the newest fifty
messages, for this week's events, or for contacts from A.

Every consumer meets it and works around it the same way. Himalaya's pimdir
backend pages the **entire** collection through the keyset cursor and sorts in
memory before it can show an envelope list; Pimalaya Linux hit the same wall the
moment its read path was due to move onto the store, and Pimalaya Android needs
date ordering identically for its calendar. A full scan per listing is
acceptable for a one-shot command and is exactly the wrong shape for an app that
repaints on every folder click: it is the one thing that would make the cached
path feel slower than the live one it replaces.

The pimdir specification now defines the fix (SPEC.md §9.3, §11, §12.1, §13).
This change implements it.

## What

- Schema: `items.sort_key TEXT NOT NULL DEFAULT ''` plus the index
  `items_by_sort ON items(collection, sort_key, seq)`. Already in
  `migrations/0001_init.sql`, which is edited in place while the spec is draft.
- Statements: `insert_item` binds `:sort_key`; `list_items_page_asc`,
  `list_items_page_desc` and `set_sort_key` are new; the read statements return
  the column.
- `PimdirItem` gains `sort_key`.
- `PimdirStore` gains the two ordered pages and `set_sort_key`. The ordered
  pages take a cursor of `(sort_key, seq)` and SHOULD accept "no cursor" for the
  first page, so a caller never has to invent a sentinel greater than every key.

## The sequencing question this leaves open

An item's key has to be *written*, and the ordinary write path into this store
is `ReplicaStorage::write`, which receives `ReplicaWriteOp::UpsertPlacement`
carrying a `ReplicaPlacement`. That placement has `meta` and no companion key,
so as things stand io-replica cannot deliver one, and this store must not go
looking inside `meta` for it (SPEC.md §9.3: that would make `meta` normative
JSON with a reserved key, for every kind, and end the property that the store
never parses it).

Two steps, in this order:

1. **`set_sort_key` first**, which this change ships. A consumer owns its meta
   convention, so it can derive keys from summaries it already wrote and restate
   them in a pass after a sync. That unblocks every consumer today without
   touching the engine.
2. **Then a companion field on `ReplicaPlacement`** (and on
   `ReplicaFetchedItem`, where a connector first derives it beside the meta), so
   the key rides the ordinary insert and the restating pass disappears. That is
   an io-replica change with its own proposal, not this one.
