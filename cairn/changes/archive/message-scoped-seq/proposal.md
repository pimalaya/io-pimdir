---
cairn: change
id: message-scoped-seq
status: landed
created: 2026-08-02
---

# The public id is scoped to the message, not the mailbox

## Why

`public-seq-id` (landed the same day) made `seq` per-collection — numbers
restarting `1..N` in each mailbox. That clashes with dedup and a merged view: the
same message filed in two mailboxes got two different ids, and id `1` meant a
different thing in every mailbox, so a cross-mailbox view could not tell them
apart. (Caught before shipping — the store is re-synced from scratch.)

## What

`seq` is now a property of the **message**: drawn from a **store-global** counter
(`store_meta.next_seq`, replacing `collections.next_seq`) and **reused** for every
placement of the same `link_id`, so a message keeps one id across every mailbox.
`insert_item` looks up an existing `seq` for the `link_id` (`SEQ_FOR_LINK_ANY`,
indexed by the new `items_by_link`) before allocating a fresh global one. Mirrors
`pimdir/SPEC.md §9.1`. himalaya needs no change (it already keys on `(collection,
seq)`).
