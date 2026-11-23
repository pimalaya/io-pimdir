---
cairn: change
id: a-purge-of-chosen-items
status: landed
created: 2026-11-19
---

# A purge of chosen items

## Why

pimdir audit-2026-11-12 P5: neverest sweeps one account's retention one synced transaction per item (src/sync/driver/upkeep.rs `purge_account`), since `purge_retained_before` is store-wide and accounts share a store. A batch in one transaction is io-pimdir's half, additive, moving nothing in the format; a sibling statement taking an account would be a pimdir minor after v1.

## What

`PimdirStore::purge_items(items)`: every `(collection, seq)` purged as `purge` does, through the canonical `PURGE_ITEM`, in one transaction, the pins released and the pages given back once, answering `PimdirPurgeReport`. neverest's `purge_account` can collect its page of retained items under the cutoff and purge them in one call.
