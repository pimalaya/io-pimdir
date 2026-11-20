---
cairn: log
change: the-crate-follows-draft-15
landed: 2026-11-20
---

# A join is added

The engine follows the sync cases pimdir pinned from its generator's engine model (pimdir cairn/log/2026-11-20-the-engine-seams-the-generator-found.md): sync vectors 181 to 186, and the format 11 `unlisted` count.

## What landed

- src/sync.rs: a member landing a create the store offered (`PimdirLoaded::offered`), the item other sources bind, joins it: reported `Added`, counted pulled, never `Created`; a body it carries is its own, its base and its summary, so the write folds it into the item by the collection's policy (SYNC §6, §9) rather than reading the source's both-changed case under its own policy (vectors 184, 185). A join carrying no body lands as before, conflicted on its binding at the listed revision.
- src/hub.rs, src/client/write.rs, src/coroutine.rs: the write answers `PimdirWritten::adopted`, the handles whose body replaced the item's (a `prefer-incoming` settle, a `PreferRemote` keep), and `PimdirWritten::refused`, those whose body `prefer-existing` kept apart. `commit` drops the `ContentChanged` of a refused one, the item keeping its own (vector 182), and reports one after a joining member's `Added` when the item adopted its body (vector 185).
- tests/engine/hub.rs: the two tests pinning a join's diverging body to the source's policy (`a_landed_create_holding_another_body_conflicts_rather_than_overwrites`, `..._follows_the_policy`) now pin the collection's: `manual` records the item's conflict, the base on b's own body; a source's `PreferLocal` settles nothing between sources; `prefer-existing` keeps the refused body and pushes the item's over it.
- tests/vectors_sync.rs: reads sync format 11 and compares the `unlisted` count with `PimdirSyncReport::unlisted` (vectors 107 and 134, both passing already).
- spec/: re-vendored from pimdir adf56bd, its generator included, and the sync vectors of the pimdir change above; tests/generated/reader.rs lists `load_round`, which that generator states, among the owner's statements no reader call wraps.

## Proof

On edbcf0b: 182 ("events differ", `ContentChanged` under `prefer-existing`), 184 (`Created` for a join) and 185 (`Created` and `Conflicted`, the source's `Manual` recording a join's diverging body) fail; 181, 183 and 186 pass. All pass after.

## Capabilities moved

sync ("An offered create is landed with the union of its flags": a join is `Added`, its carried body folded by the collection's policy; the events requirement: `ContentChanged` names a body the item took; "A capped listing proves no absence": the count is a MUST, pinned by vectors).

## Left

- A join carrying no body, recorded here as a binding conflict the upgrade clears on an equal body, where pimdir SYNC §9 records an item conflict no rule clears on an equal body: waits for the owner (pimdir log above).
- android/rust/src/offline.rs builds `PimdirWritten` by a struct literal and needs `..Default::default()` (or the two lists) when it takes this crate.
