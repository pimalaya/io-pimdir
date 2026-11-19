---
cairn: tasks
change: a-purge-of-chosen-items
---

- [x] `PimdirStore::purge_items` in src/client.rs
- [x] Test: tests/retention.rs `purge_items_purges_the_retained_ones_in_one_write` (live, missing and foreign ids skipped, pins released, bodies collected)
- [x] cairn/spec/store.md folded, log, CHANGELOG
- [ ] neverest's `purge_account` calls it (neverest's to do)
- [ ] Asked by neverest on 2026-11-19: an account-scoped batch purge (the retained items of one account before a cutoff, one transaction), so neverest need not list them; io-pimdir can build it from `list_retained` and `PURGE_ITEM` without a format change, or pimdir adds a sibling statement in a minor after v1
