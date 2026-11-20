---
cairn: log
change: the-crate-follows-draft-15
landed: 2026-11-20
---

# An equal body is no divergence

The engine follows pimdir's sync-draft-15 rules of cairn/log/2026-11-20-an-equal-body-is-no-divergence.md there: a join carrying no body is the item's conflict, which a body read equal to the item's clears.

## What landed

- src/sync.rs: a mutable member joining an item the store offered, carrying no body, holds none: its placement names no body, its base the listed revision and no body, and the write records the item's conflict. It was recorded on its binding under the source's policy (vector 187).
- src/hub.rs `PimdirHubItem::reconcile`: a source binding none of the item before the upsert changed, so a joining upsert carrying no body over an item holding one is the unread divergence `manual` records whatever the policy; and an item conflict holding no `conflict_object` clears when the binding whose base named no body reads one equal to the item's, its base and `shared_object` taking it (vector 188).
- spec/: re-vendored from pimdir 957ebcd (its generator's count and bytes profiles and engine histories included), with the sync vectors of the pimdir change above.

## Proof

On 8e09e1c, vectors 187 ("bindings rows differ", the join conflicted on its binding) and 188 (the item left conflicted over two equal bodies) fail; both pass after. Vector 182, now with `push` true, passes on both: the engine already left what its fold owes to the next run. Vector 133, now stating the item and binding unconflicted after an unread base is read outside an item conflict, passes on both.

## Capabilities moved

sync ("An offered create is landed with the union of its flags": a mutable join carrying no body is the item's conflict; "A change is derived before its write folds it", added), upgrade ("An item conflict's late body meets the policy": an equal body clears it).
