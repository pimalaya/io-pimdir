---
cairn: log
change: the-agreed-body-stays-pinned
date: 2026-10-30
---

# The agreed body stays pinned

pimdir sync-draft-05's merge (a-merge-keeps-every-body, a-late-body-meets-the-policy): the agreement point is pinned, no body is dropped without being kept, and a body nobody has read is never chosen against. Closes the release audit's blocker 6 (the hub dropping a local edit over an emptied item) and S14 (a `PreferRemote` body-less pull reversed).

## What landed

- src/hub.rs: `reconcile_content` reads an item holding no body as changed for a binding that agreed one; a body filling an emptied item from a binding whose base names none is adopted, an item conflict standing; a divergence one of whose bodies is unread is recorded as `manual` whatever the policy; `prefer-incoming`, `prefer-existing` and a `PreferRemote` pull dropping a local edit hand their loser to `PimdirHub::overruled`; `shared_object` moves only with a body the item settled on; an item keeping its body keeps its summary, sort key and level.
- src/client/write.rs: the loser kept as a retained item under `dup:<link>#<handle>`, minted again over a held key, a `seq` of its own, deleted, retained by the settling source, its body pinned; `PimdirSourceStore::write` answers `PimdirWritten`.
- src/sync.rs: `Conflicted` in place of the `ContentChanged` of a pull the write recorded as the item's divergence, `Overruled` for each kept body. src/upgrade.rs: an item conflict is refetched only by a binding whose base names a revision and no body, its body landing as the binding's own for the write to settle by policy; `PimdirUpgradeReport::events`. src/mutate.rs: `PimdirMutateReport::events`.
- tests/engine/hub_property.rs: the ledger moves no agreement point on an upsert leaving the item conflicted (an edit, a flag change, a sync, a rekey's carry), and the divergence oracle reads the item's body and the source's agreement point, or its sync base before it folded once, from the hub before each edit: a ledger kept from the ops alone lost the point where a binding folds while the item is conflicted. The rule it checks: a source that agreed with the body the item holds never diverges, and one that did not, staging another body, always does. Its four seeds of 2026-10-30 in tests/proptest-regressions/hub_property.txt read the old rule, under which a source re-editing a conflicted item silently replaced the other's body.
- Failing first: vectors 14, 85, 86, 90, 91 ("events differ"), 28, 33, 80, 81, 82, 84 ("bindings rows differ"), 83 ("events differ"); src/hub.rs `a_local_edit_over_an_emptied_item_is_recorded` against the old merge ("the edit is a divergence, not dropped").
- Dropped: clearing `items.conflicted` when a source restates the shared body. SYNC §3 and §7 settle an item conflict by an `Edit` or a `Remove`; a source restating the shared body says nothing about the other's diverging body.
- Capabilities moved: hub ("A binding records what it last agreed with the hub on", "The hub resolves cross-source content conflicts by policy", modified), sync ("Headless conflict resolution", "Events report what the remote changed", "Per-item delta events", modified), upgrade ("An item conflict's late body meets the policy", added), seam ("A write answers what the merge recorded", added), mutate (the report).

## For neverest

Its in-memory divergence workaround (src/offline/driver.rs:3478-3866) and its diffing of `items.conflicted` around every pass (driver.rs:3557-3570) can go once it ports to this revision: the store pins the ancestor and records the divergence itself.
