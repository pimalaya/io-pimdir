---
cairn: tasks
change: acknowledged-intents-leave-receipts
---

- [x] Re-vendor spec/ from pimdir (three statement comments).
- [x] `PimdirStore::acknowledge_action`; `replace_action` records the intent's receipt; one `record_receipt` shared with the drain.
- [x] tests/receipts.rs: acknowledged, dropped, parked then acknowledged, replaced.
- [x] Fold into cairn/spec/store.md; log; CHANGELOG.
