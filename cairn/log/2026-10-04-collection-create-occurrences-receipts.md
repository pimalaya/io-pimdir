---
cairn: log
change: collection-create-occurrences-receipts
date: 2026-10-04
---

# Collection creation, occurrence intents and queue receipts

pimdir draft-04 adds the `collection-create` intent, an optional `recurrence_id` in the invitation intents and queue receipts; this crate follows, spec/ re-vendored. Change folder: cairn/changes/collection-create-occurrences-receipts.

## What landed

- src/capability.rs: `COLLECTION_CREATE` in the mail, contacts and calendar lists, `CALENDAR_REPLY_OCCURRENCE`, `CALENDAR_CANCEL_OCCURRENCE`, `intent_capability("collection-create")`.
- src/intent.rs (I/O-free): typed payloads of `collection-create`, `calendar-reply` and `calendar-cancel`, strict reading, `recurrence_id` validation; `PimdirActionError::Invalid`.
- src/client/capability.rs: the gate checks a `collection-create`'s anchor and refuses it without a declared performer; an occurrence needs its capability from the performer and is refused on an undeclared account.
- src/client/producer.rs: `enqueue_collection_create`; receipts recorded by the drain, pruned after its pass; `PimdirActionStatus`, `action_status` on the producer and the reader. src/client/schema.rs: `receipts` reconciled on open.
- CLI: `queue status`.
- Tests: unit tests in src/intent.rs; tests/capabilities.rs (three scenarios); tests/receipts.rs (five).

Decided here: receipts pruned after the drain's pass rather than before, so the drain still reads its pending list before any write of its own (tests/queue.rs, a claim that deletes nothing).

## Capabilities moved

- store: three requirements added (collection-create, occurrences, receipts), the reconcile on open extended to `receipts`.
- cli: `queue status` in the verb surface.
- spec-fidelity: names draft-04.
