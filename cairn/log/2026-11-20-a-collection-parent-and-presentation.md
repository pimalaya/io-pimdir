---
cairn: log
change: the-crate-follows-draft-15
landed: 2026-11-20
---

# A collection's parent and presentation

pimdir storage-draft-15 gained the owner statements `set_collection_parent` and `set_collection_presentation` (pimdir cairn/log/2026-11-20-a-collection-parent-and-presentation.md); the crate wraps them. Before, no statement wrote `collections.parent`, `color`, `description` or `sort_order`, and the crate only read them.

## What landed

- src/client.rs: `PimdirStore::set_collection_parent(collection, parent)`, refusing a parent breaking the account's forest as `PimdirError::Parent` (`collection-parent`) and a collection or parent the store does not hold as `PimdirError::Undeclared` (`collection-undeclared`), told apart by the change count and a `load_kind` read; `PimdirStore::set_collection_presentation(collection, color, description, sort_order)`, an undeclared collection refused as `Undeclared`. Both variants are additive, `PimdirError` being `non_exhaustive`.
- src/client/write.rs: `collection_exists`.
- src/cli/error.rs: the `pimdir` binary relays both codes.
- tests/collection_parent.rs: the forest refusals (a cycle, itself, another account), an unknown parent and collection, clearing; the presentation written whole, ordering the listing, cleared, refused on an undeclared collection.
- spec/queries/: the two statements and queries/signatures.json, vendored from the pimdir working tree of this change.

## Capabilities moved

store ("A collection keeps the parent and presentation its source states", added).
