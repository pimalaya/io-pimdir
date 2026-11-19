---
cairn: log
change: a-purge-of-chosen-items
landed: 2026-11-19
---

# A purge of chosen items

`PimdirStore::purge_items` purges the retained items a list names in one transaction, through the canonical `PURGE_ITEM`, the pins released and the pages given back once (src/client.rs). io-pimdir's half of pimdir audit-2026-11-12 P5; neverest's sweep moving to it is neverest's. Additive, nothing in the format.

Capabilities moved: store (retention: `purge_items`, and the vacuum after it).

Test: tests/retention.rs `purge_items_purges_the_retained_ones_in_one_write`. A new method, so no test could fail against the old code but by not compiling.
