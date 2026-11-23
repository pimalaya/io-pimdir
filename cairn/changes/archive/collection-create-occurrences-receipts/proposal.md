---
cairn: change
id: collection-create-occurrences-receipts
status: landed
created: 2026-10-04
---

# Collection creation, occurrence intents and queue receipts

> Cross-repo change, same id in pimdir (STORAGE §4.3, §13, §14.1, §15.1, §15.2, §15.4, Annex B, migration 0001, four statements, draft-04), which lands first; this crate re-vendors spec/ and follows. The rationale is pimdir/cairn/log/2026-10-04-collection-create-occurrences-receipts.md.

## Why

A client of the queue (MOA, through himalaya and neverest) could not ask for a folder on the server, could not answer or cancel one occurrence of a series, and could not learn the item its `add` created once applied: it matched the body against the store by Message-ID.

## What

- The `collection-create` intent and the `collection.create` capability, declared for every kind; a typed payload, the gate checking its anchor, and `PimdirProducer::enqueue_collection_create`.
- An optional `recurrence_id` in `calendar-reply` and `calendar-cancel`, gated on `calendar.reply.occurrence` and `calendar.cancel.occurrence` from the same performer, refused against an account that declares nothing.
- Receipts: the drain records one per applied row, `seq` set for an `add`, and prunes those past seven days; `PimdirProducer::action_status` and `PimdirReader::action_status` follow a row by id; the table reconciles on open; `pimdir queue status`.
