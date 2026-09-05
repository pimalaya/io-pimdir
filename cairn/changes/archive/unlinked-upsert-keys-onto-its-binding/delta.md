---
cairn: delta
change: unlinked-upsert-keys-onto-its-binding
---

# Delta

## ADDED Requirements

### Requirement: An unlinked upsert lands on the binding its handle holds
An `UpsertPlacement` carrying no `link_id` SHALL be resolved against the binding its `(collection, source, handle)` holds (`LINK_FOR_HANDLE`) and folded through the hub as that item. Only a handle no binding holds SHALL stage unlinked, as the freshly probed row it is, awaiting the `Meta` upgrade that names it.

A placement carries no link id because its identity has not been read, not because it has none. A handle the store has already bound has one, and staging a second row for it makes `load` answer with two placements for one handle, which is read one layer up as two items: the upgrade fetches the handle twice and mints a `dup:` key for a copy that does not exist, binding one source handle to two items no sync can converge. The write is ordinary, not exotic: a remote edit of a locally deleted item is pulled as a fresh probe of the handle the tombstone still binds (SPEC §10).

Resolving before the fold is also what puts such an upsert under the identity floor: the rebind guard and the link set a batch folds into both read the placement's link id, and neither sees a placement that has none.

#### Scenario: A reprobed handle stays one item
- GIVEN a handle bound to an item
- WHEN a write upserts a placement for that handle carrying no link id
- THEN it folds into the bound item and the handle answers with one placement

#### Scenario: A handle nothing holds is still a probe
- GIVEN a handle no binding holds
- WHEN a write upserts a placement for it carrying no link id
- THEN it stages unlinked and claims no identity
