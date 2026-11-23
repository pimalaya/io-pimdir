---
cairn: change
id: an-item-keeps-local-state
status: landed
created: 2026-11-21
---

# An item keeps local state

## Why

pimdir daec31a (storage-draft-15 and search-draft-16, edited in place) gives an item local state no source carries: `item_property(seq, key, value)`, keyed by the public id, a local label (`pimdir/label`) and an application's own state (`himalaya/snooze-until`) among it, applied by the owner from a new `set-property` action and read by the search fields `label:` and `property:` (pimdir cairn/log/2026-11-21-an-item-keeps-local-state.md). The crate follows the storage half here; the two query fields wait on the query planner of as-fast-as-notmuch, which rewrites the compiler now.

## What

- spec/ re-vendored at pimdir daec31a: the table, its index, CHECKs and five triggers, the four reads and two writes, `evict_before` keeping an item holding a property, the `set-property` schema and vectors, store.json and refresh.json at `v` 3.
- `io_pimdir::property`, I/O-free: the bounds, the key and value rules, the label fold (Annex A.2's `fn` fold) and `change`, the one fold of a `set-property` over the values held that the owner, the drain and the overlay share.
- `PimdirAction::SetProperty` and its v1 codec, the value's 1,024 bytes the text's rule beside the schema's characters.
- The owner's apply: as no source, no capability, a receipt naming the item, parked when no live row of its collection holds the seq or past 64 values.
- The reader's `list_item_properties`, `list_property_values`, `list_property_holders`, `list_value_holders`, and `item_properties`, the overlay of a pending `set-property` (§15.4); the owner's `add_property` and `remove_property` on its own account, refused as `PimdirError::Property` (`property-invalid`) past a bound.
- The `pimdir property list|add|remove` verbs, staged through the queue as `tag` stages its flags.
- tests/search.rs seeds `properties` and applies the refresh `property` write; the `label:` and `property:` vector cases are on tests/pending_vectors.txt until the planner merge.
