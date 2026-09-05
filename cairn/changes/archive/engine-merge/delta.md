---
cairn: delta
change: engine-merge
---

# Delta

## ADDED Requirements

The capability files coroutine, seam, sync, upgrade, mutate, rekey and hub, folded from io-replica with the four rule changes the pimdir spec settled, and the summaries capability replacing conventions. In store:

### Requirement: A summary and its addresses are written with the item
A named placement carries its `PimdirSummary` (STORAGE Annex A), and the write SHALL persist it as the row of the kind's summary table and the `item_address` rows, in the item's transaction, only when it moved: the batch's load reads the summaries and addresses of the link ids it names, the diff compares them, and a summary that moved under unmoved item columns runs `stamp_item` so the change feed sees it (STORAGE §4.5). A summary of another variant deletes the old row; a placement carrying none deletes the stored row too.

### Requirement: Probes are rows
A pulled member whose identity is not read yet SHALL be a row of the `probes` table rather than an in-memory residual, so a first enumeration survives a crash and a reader can count what a source enumerated but nothing has identified.

### Requirement: The change feed is the triggers'
`items.changed` and `collections.changed` SHALL be stamped by the canonical triggers and never bound by a writer; a purge counts in `store_meta.purges`. The reader SHALL expose `change_cursor`, `items_changed_since` and `collections_changed_since` (STORAGE §4.5).

### Requirement: The refcount floor is a constraint
`objects.refcount` SHALL carry `CHECK (refcount >= 0)`, so a double release fails at the statement that caused it (STORAGE §7).

### Requirement: A rekeyed batch bumps the generation
A batch carrying a `Rekeyed` drop SHALL bump the collection's generation in the same transaction, the engine emitting no op for it.

### Requirement: A store from an earlier draft is refused
A store stamped 1 by an earlier draft, lacking a table the schema declares, SHALL be refused with `PimdirError::Stale` naming the table: the operator deletes it and lets it resync. No migration is offered while the format is a draft.

## MODIFIED Requirements

### Requirement: An item is retained, never deleted, when its last binding goes
The hub keeps an unbound item and the store retains it: the diff releases only the bindings' references, the row's own stay counted, and garbage collection never sweeps a retained body.

### Requirement: Events are pull-side only
An accepted push reports nothing, as the spec's sync vectors decided.

### Requirement: A superseded handle licenses its own rebind
`Rekeyed` joins `Superseded` as a drop reason letting the binding move.

### Requirement: A write reads only the rows its batch names
The batch's load reads the summaries and addresses by link id beside the item columns.

### Requirement: The queue derives summaries from bodies
A queued `add` or `update` SHALL carry no summary: the drain reads the body the producer wrote and derives the key, the summary and the sort key under Annex A for the collection's declared kind, parking an `add` that names no link id and derives none.

## REMOVED Requirements

### Requirement: The draft shape is reconciled on open
Gone with the unreconcilable-store refusal: a store from an earlier draft is refused as stale, never rebuilt.

### Requirement: The residual survives in memory
Gone: probes are rows.

### Requirement: `write_rekeyed`
Gone: a rebuild is an ordinary batch carrying `Rekeyed` drops.

### Requirement: The raw `meta`
Gone: an item carries a typed summary and its address rows.
