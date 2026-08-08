---
cairn: log
change: carry-a-sort-key
date: 2026-08-08
landed: 2026-08-08
---

# The store binds the sort key io-replica now carries

io-replica put a sort key on `ReplicaPlacement`, so the store binds it: `load`
returns it, `INSERT_ITEM` and `UPDATE_ITEM` write it, and `ReplicaHubItem`
carries it through the hub.

**This reverses the arrangement that landed hours earlier, and the reversal is
the point.** When nothing upstream carried a key, `UPDATE_ITEM` preserved the
stored one by never naming it, and `LOAD_ITEMS` deliberately did not select it,
documented as a permitted substitution: the reference save is a replace-all and
has to carry the key back out through `load`, while a diffed save does not.

That reasoning was correct and is now obsolete. With a key on the placement, a
`load` that drops it hands every save an unknown key, and an update that binds
it writes that back, blanking on every sync exactly what the previous sync
derived. The two halves have to move together: either both ignore the key or
both carry it. They now both carry it, which also puts `LOAD_ITEMS` back in step
with the canonical statement and removes one entry from the substitution list.

Pinned by a test that writes a placement with a key, loads the collection, and
writes back exactly what it loaded, asserting the key is still there. Without
the `load` change that test fails, which is the failure that would otherwise
have reached a user as a list that scrambles itself every sync.

A queued action carries no key: a queue producer is not a connector, so it
derives none, and the sync that pushes the create resolves one.

`io-replica` is a path dependency for now, since the field is unreleased.

Capabilities moved: **store** (the key is bound on write and returned by load).
