---
cairn: change
id: capabilities
status: landed
created: 2026-10-03
---

# Capabilities

Self-contained: a session with no prior context can take it from here.

## Why

pimdir STORAGE §15.6 and Annex B (pimdir change `capabilities`, draft-03 pending) add declared capabilities: the owner says what each source can do, a producer refuses an action a declared source does not support before it is queued, an intent names the one source performing it, and the owner parks what slipped through. io-pimdir is where producers and owners meet the store, so the gate lives in `enqueue` and every CLI gets it without code of its own.

## What

- `capability` (I/O-free): the Annex B vocabulary (`MAIL`, `CONTACTS`, `CALENDAR`), `PimdirSupport`, `PimdirCapability`, `required` (by action and flags), `required_by_content` (calendar: `scheduling`, `occurrence.update`, `online-meeting` read from the resources), `check`, `intent_capability`.
- `summary::calendar`: `scheduled`, `occurrences`, `online_meeting` over the existing iCalendar parser.
- Schema: `capabilities` (with `account`) and `performers`, created on open in a 0.5 store.
- `PimdirStore::declare`; `PimdirProducer::check`, `performer(collection, capability, chosen)`, `capabilities`; `PimdirReader::capabilities`, `item_capabilities`, `performers`.
- `enqueue` runs the gate, reading the new body from the blob store and the item's current one; the drain re-runs it as the backstop and parks a refusal; `set-performer` is applied by the drain.
- An intent's candidates are read at its anchor collection, a collection row winning over the source-wide one, so a provider's reply reaching only its own calendars and an iMIP one reaching any are both candidates where both apply.
- `PimdirError::Unsupported`, `NoPerformer`, `Ambiguous`.
