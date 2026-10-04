---
cairn: change
id: acknowledged-intents-leave-receipts
status: landed
created: 2026-10-04
---

# An acknowledged intent leaves a receipt

> Cross-repo change, same id in pimdir (STORAGE §4.3, §13, §15.4, §15.5, Annex B.2; comments of `record_receipt`, `cancel_action`, `load_receipt`), which lands first; this crate re-vendors spec/ and follows.

## Why

An intent is performed by its owner out of band (neverest sends a `submit`, answers a `calendar-reply`, creates a `collection-create`) and acknowledged with `drop_action`, which records no receipt. Once the row is gone a producer reads a message sent or a folder created as `Unknown`, exactly like a row cancelled by request.

## What

- `PimdirStore::acknowledge_action(id, seq)`: deletes the row, pending or parked, releases its pin and records its receipt (the row's own collection, `seq` the item the intent left when the performer knows it) in one transaction; `false` and nothing recorded when the row is gone.
- `replace_action` records the replaced intent's receipt, `seq` `None`.
- `drop_action` stays the withdrawal: no receipt.
