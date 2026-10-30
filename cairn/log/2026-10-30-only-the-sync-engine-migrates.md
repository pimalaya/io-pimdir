---
cairn: log
change: only-the-sync-engine-migrates
date: 2026-10-30
---

# Only the sync engine migrates

A consumer cancelling a queue row, and the operator CLI's owner verbs, ran the migrating owner open, so an older binary rebuilt a newer store to its own schema and dropped what it did not know (himalaya at 3b4bed8 dropped `supersessions` and four `task_summary` columns). They now take the store as it stands.

## What landed

- src/client.rs: `PimdirStore::cancel_action` opens through `open_current`; a store of another version or shape is refused as `PimdirError::NotCurrent`, left as it was; an absent one stays `Uncreated`.
- src/cli/mod.rs: `StoreFlags::owner` and `owner_if_free` open through `open_current`. `queue cancel`, `gc`, `item purge` and `check --fix` fail with `NotCurrent`'s sentence over a store of another revision; `item restore` and `tag` leave their action queued and report it `queued`. src/cli/item.rs, src/cli/search.rs: the `queued` statuses say so.
- No other non-engine path runs `open` (`open_with_hash` is called by `open` alone).
- Tests: tests/open_current.rs `a_cancel_leaves_a_store_needing_a_reconcile_as_it_was`; tests/cli_owner.rs (new) `an_owner_verb_leaves_a_store_of_another_revision_as_it_was`, `a_restore_over_a_store_of_another_revision_stays_queued`. The first two failed on 4c3353e (the cancel succeeded and reconciled; `queue cancel` exited 0).
- Capabilities moved: store ("A queued action can be cancelled or acknowledged", modified), cli ("Terminal operations take the owner role directly", modified).
