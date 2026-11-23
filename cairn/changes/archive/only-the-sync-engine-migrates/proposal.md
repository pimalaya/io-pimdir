---
cairn: change
id: only-the-sync-engine-migrates
status: landed
created: 2026-10-30
---

# Only the sync engine migrates

## Why

`PimdirStore::open` creates, migrates and reconciles the store to this crate's schema (STORAGE §6), dropping the tables and columns it does not know at an unchanged `user_version` 1. `PimdirStore::cancel_action` (src/client.rs:877-885) runs it, so a frontend cancelling one queue row rebuilds the store to its own revision: himalaya at 3b4bed8 dropped `supersessions` and four `task_summary` columns from a store a newer neverest kept (report-io-pimdir.md, blocker 4). The CLI's owner verbs (src/cli/mod.rs:101, :108: purge, queue cancel, gc, check `--fix`, restore, tag) run it too, so an older `pimdir` binary does the same to a newer store.

Only the program that syncs the store (its owner engine) should ever bring it to its schema. A program owning the store for one write takes it as it stands, `open_current`, which refuses a store it would have to rebuild (unfog's time collection already follows this pattern).

## What

- `PimdirStore::cancel_action` opens through `open_current`: a store of another version or shape is refused as `PimdirError::NotCurrent` and left as it was. An absent store stays `PimdirError::Uncreated`.
- The CLI's owner role (`StoreFlags::owner`, `owner_if_free`) opens through `open_current`; `NotCurrent` reports as the plain sentence its `Display` gives (its owner has to migrate or reconcile it first), and a verb that enqueues then drains (`item restore`, `tag`) leaves its action queued, as when another process owns the store.
- No other non-engine path in the crate runs `open` (checked: `open_with_hash` is called by `open` alone).
