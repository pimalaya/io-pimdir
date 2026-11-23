---
cairn: tasks
change: the-agreed-body-stays-pinned
---

- [x] pimdir storage-draft-05 / sync-draft-05 landed (spec edit 4: shared body pin, `prefer-*` loser)
- [x] Test: two sources, a bodiless pull through one, a local edit through the other: the edit is recorded, never `Clean` (src/hub.rs `a_local_edit_over_an_emptied_item_is_recorded`, failed first on the old merge: "the edit is a divergence, not dropped"; vector 81 the same through an upgrade)
- [x] Test: a bodiless pull under `PreferRemote` keeps the server edit (S14): vector 83 (failed first: "events differ", the item kept `c3` unflagged, so the next run pushed it over the unread edit)
- [x] src/hub.rs: the merge with no shared body (fill, unread divergence recorded as `manual`), the agreement point pinned while the item is conflicted or the upsert carries no body
- [x] src/client/write.rs: `shared_object` pinned, released when the binding agrees with another body (the-store-follows-storage-draft-05)
- [x] `prefer-*` and `PreferRemote`: the losing body kept as a retained item, `Overruled` reported once the write commits (vectors 14, 85, 86, failed first: "events differ")
- [x] The late body of an item conflict meets the policy at the upgrade (vectors 84, 90, 91)
- [x] `items.conflicted` cleared on a restated shared body: dropped, not a rule. SYNC §3 and §7 settle an item conflict by an `Edit` or a `Remove`; a source restating the shared body says nothing about the other's diverging body, which clearing would discard
- [x] tests/vectors_sync.rs: vectors 14, 28, 33, 80 to 86, 90, 91
- [x] tests/engine/hub_property.rs: the ledger moves no agreement point on an upsert leaving the item conflicted, and the divergence oracle reads the item's body and the source's agreement point (or sync base) from the hub before each edit, a ledger kept from the ops alone losing the point where a binding folds while the item is conflicted (its four seeds of 2026-10-30 read the old rule)
- [x] Spec: hub.md, sync.md, upgrade.md, mutate.md; fold, log, CHANGELOG
- [x] Tell neverest its divergence workaround (driver.rs:3478-3866) can go: in the wave report of 2026-10-30
