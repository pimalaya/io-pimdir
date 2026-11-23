---
cairn: delta
change: collection-create-occurrences-receipts
---

# Delta

## ADDED Requirements

### Requirement: A collection-create is anchored on its parent and needs a declared performer
(store) The gate SHALL read a `collection-create` payload strictly (`PimdirCollectionCreate::from_payload`: `v: 1`, a non-blank `name` without control characters, an optional `parent`), refuse one naming a `parent` other than the collection it is enqueued on or anchored on a collection of no declared kind (`PimdirActionError::Invalid`), and refuse it with `NoPerformer` on an account whose sources declare nothing. `PimdirProducer::enqueue_collection_create` SHALL anchor it on its parent or the collection given, resolve the performer and write it into the payload.

### Requirement: An occurrence is gated beside its intent
(store) A `calendar-reply` or `calendar-cancel` naming `recurrence_id` SHALL be refused unless the value is `YYYYMMDD` or `YYYYMMDDTHHMMSS[Z]`, and unless the performer declares `calendar.reply.occurrence` or `calendar.cancel.occurrence` at the anchor; on an account whose sources declare nothing it SHALL be refused, since the owner would widen it to the series.

### Requirement: An applied row leaves a receipt a producer follows
(store) The drain SHALL record a receipt in the transaction applying a row, with the `seq` an `add` created, and prune the receipts older than seven days after its pass; `action_status(id)` SHALL answer pending, parked with its error, applied with the receipt, or unknown. The `receipts` table SHALL reconcile on open, and its absence reads as no receipt.

### Requirement: The verb surface
(cli, modified) `queue status`: where one action stands by id.

## MODIFIED Requirements

### Requirement: The vendored copy is the specification's, byte for byte
(spec-fidelity) The text copied is `draft-04`.
