---
cairn: tasks
change: an-unreadable-body-is-retried-then-parked
---

- [x] Test: a queued `add` whose blob is gone is retried, then parked past `MAX_ATTEMPTS`, and never creates the item (fails on 4c3353e: the add applies)
- [x] Test: the same for an `update` (fails on 4c3353e: the update repoints the item to the missing body)
- [x] src/client/producer.rs: `stage_action` fails an `add` or `update` whose body is missing as an I/O failure
- [x] Spec: store.md; fold, log, CHANGELOG

The pimdir §15.2 vector (spec edit 4) is run under a-queued-edit-names-what-it-changes once it lands.
