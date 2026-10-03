---
cairn: log
change: capabilities
date: 2026-10-03
---

# Capabilities

pimdir draft-03 adds capabilities (STORAGE §15.6, Annex B); this crate follows, spec/ re-vendored.

## What landed

- src/capability.rs (I/O-free): the vocabulary, `required` by action and flags, `required_by_content` reading a calendar resource, `check`.
- src/summary/calendar.rs: `scheduled`, `occurrences`, `online_meeting` over the existing parser.
- src/client/capability.rs: declaration rows, the gate run by `enqueue` and by the drain, performer resolution with a queued choice read first, `set-performer`.
- `PimdirStore::declare`, `replace_action`; `PimdirProducer::check`, `performer`, `capabilities`; `PimdirReader::capabilities`, `item_capabilities`, `performers`; schema reconcile on open.
- Tests: tests/capabilities.rs (ten scenarios), unit tests of the gate; neverest, himalaya, cardamum and calendula built and run end to end against it (pimdir cairn/changes/capabilities/e2e.md).

## Capabilities moved

- store: five requirements added, from the gate to the reconcile on open.
- spec-fidelity: names draft-03.
