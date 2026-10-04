---
cairn: log
change: acknowledged-intents-leave-receipts
date: 2026-10-04
---

# An acknowledged intent leaves a receipt

pimdir draft-04 (edited in place, untagged) now has a performer record an intent's receipt when it acknowledges it, so a producer tells a message sent or a collection created from a row withdrawn; this crate follows, spec/ re-vendored (three statement comments). Change folder: cairn/changes/acknowledged-intents-leave-receipts.

## What landed

- src/client.rs: `PimdirStore::acknowledge_action(id, seq)`, the row's collection read in the transaction (`load_action`); `replace_action` records the intent's receipt, `seq` `None`; `drop_action` unchanged, now documented as the withdrawal; one `record_receipt` helper shared with the drain (src/client/producer.rs).
- `PimdirActionStatus` and `pimdir queue status` documentation name performed intents.
- Tests: tests/receipts.rs, three scenarios (acknowledged and dropped, parked then acknowledged with a `seq`, replaced with its change followed apart).

Decided here: the collection comes from the row rather than from the caller, so an acknowledgement cannot file a receipt under another collection.

## Capabilities moved

- store: the cancel-or-acknowledge requirement splits withdrawal from acknowledgement; `replace_action` and receipts requirements name the intent's receipt.
