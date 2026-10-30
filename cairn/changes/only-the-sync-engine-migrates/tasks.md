---
cairn: tasks
change: only-the-sync-engine-migrates
---

- [x] Test: `cancel_action` on a store needing a reconcile is refused and the store left as it was (fails on 4c3353e)
- [x] src/client.rs: `cancel_action` through `open_current`
- [x] Test: `pimdir queue cancel` and `pimdir gc` on a store needing a reconcile fail and leave it as it was (fails on 4c3353e)
- [x] Test: `pimdir item restore` over such a store leaves its action queued, the store as it was
- [x] src/cli/mod.rs: `owner` and `owner_if_free` through `open_current`, `NotCurrent` leaving the work queued in `owner_if_free`
- [x] Spec: store.md, cli.md; fold, log, CHANGELOG
