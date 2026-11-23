---
cairn: tasks
change: collection-create-occurrences-receipts
---

# Tasks

- [x] Land the pimdir side (draft-04) in the sibling ../pimdir checkout and re-vendor spec/.
- [x] `capability`: `COLLECTION_CREATE` in every domain's list, `CALENDAR_REPLY_OCCURRENCE`, `CALENDAR_CANCEL_OCCURRENCE`; `intent_capability("collection-create")`.
- [x] `intent` module: `PimdirCollectionCreate`, `PimdirInvitation`, `PimdirIntentItem`, `PimdirPartstat`, `recurrence_id`, `validate_recurrence_id`; `PimdirActionError::Invalid`.
- [x] Gate: a `collection-create` anchored on its parent and on a collection of a declared kind, refused without a declared performer; an occurrence needs its capability from the performer, refused on an undeclared account.
- [x] `PimdirProducer::enqueue_collection_create`.
- [x] Drain: `record_receipt` in the applying transaction, `prune_receipts` after the pass (`RECEIPT_DAYS`); `action_status` on the producer and the reader; `receipts` reconciled on open.
- [x] CLI `queue status`.
- [x] Tests: src/intent.rs, tests/capabilities.rs (three scenarios), tests/receipts.rs (five).
- [x] Fold into spec/store.md, spec/cli.md, spec/spec-fidelity.md; log.
