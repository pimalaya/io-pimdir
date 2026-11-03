---
cairn: change
id: the-crate-follows-the-freeze-gaps
status: landed
created: 2026-11-03
---

# The crate follows the freeze gaps

## Why

pimdir cairn/log/2026-11-03-the-freeze-gaps-fold-into-draft-08.md closes the last open sync rules before the pin, folded into storage-draft-08 and sync-draft-07 without a bump, with vectors 131 to 135. 132 to 135 failed here (131 passed): an edit over a base naming no body was pushed over a body nobody read, a capped listing deleted what the cap left out, and a bodiless copy into a collection its source does not sync was staged for ever.

## What

- An edit over a base naming a revision and no body derives no `Update`, nor the `Remove` a move rides behind it; an upgrade at `Full` fetches that base whatever the level.
- `PimdirRemoteSnapshot::capped`: a capped round finds no member absent, counted in `PimdirSyncReport::unlisted`. **Breaking** for a struct literal.
- `PimdirMutate::syncs_target`, set by `PimdirSourceStore::mutate` from `collection_sources`: a `Copy` or a `Move` of a placement holding no body fails `Undeliverable` unless its binding is based and its source syncs the target.
- spec/ re-vendored at pimdir 6687cf4; the vector runner reads a Copy's and a Move's `to`, as the schema names it.
