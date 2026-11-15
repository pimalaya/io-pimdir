---
cairn: change
id: a-pending-create-is-withdrawn-whatever-the-rights
status: active
created: 2026-11-15
---

# A pending create is withdrawn whatever the rights

## Why

Follows pimdir's change of the same id (pimdir cairn/changes/a-pending-create-is-withdrawn-whatever-the-rights). Live, 2026-11-15: a message himalaya added to a Graph folder other than Drafts was applied by the drain and rejected by every sync, a pending create of the Graph source; deleting it (a move to the trash) was refused by this crate's gate, "Source microsoft/msgraph does not support mail.message.move: item.create or item.delete is disabled". `capability::at_item` read every source binding the item, the pending create's included, for the `Remove`, `Move` and `Copy` arm of `gate_by_kind` and for a removed calendar resource's content gate, though the remove only withdraws the create (mutate.rs drops a tombstone's binding that has no base) and pushes nothing.

## What

1. Re-vendored `load_item_capabilities`, which answers `held` last.
2. `capability::holding`: the sources binding the item whose binding has a base, `held` 1. The gate reads it for a `remove`, a `move` and a `copy`, so a source holding the item as a pending create alone is not asked its remove nor its move; for a move or a copy that source, when it syncs the target, is among the target's sources binding nothing of the item, asked an `add`'s capabilities with the item's flags. A removed calendar resource's content needs (`calendar.scheduling`) read the same sources.
3. `at_item` and `PimdirReader::item_capabilities` keep answering every source binding the item; a `set-flags` and an `update` still read them, what they change riding on the add.

## Decided

- **No change to `PimdirSourceCapabilities`.** A public struct built as a literal; a `held` field would break a consumer building one. The gate reads the column through a private function instead.
