---
cairn: tasks
change: capabilities
---

# Tasks

- [x] Vocabulary, support, rows, `required`, `check` (src/capability.rs), unit tests.
- [x] Tables reconciled on open; `declare`, gate in `enqueue`, backstop and `set-performer` in the drain (tests/capabilities.rs).
- [x] Declaration rows carry the account; a declared source is a candidate before it syncs.
- [x] Calendar content rules (`required_by_content`, summary::calendar scanners), unit test for occurrences, integration tests for scheduling, online meeting, and native versus iMIP reply candidates.
- [x] Built against by neverest, himalaya, cardamum and calendula through `[patch.crates-io]`; their suites pass.
- [x] `mail.submit.copy`; a `submit` naming `copy` refused when its performer cannot file it; `PimdirStore::replace_action` (tests/capabilities.rs).
- [x] Intents checked once their account has declared anything.
- [x] A performer choice holds while queued: `set-performer` pending read over the recorded one (`chosen`, producer and reader).
- [x] Fold delta.md into cairn/spec/store.md, CHANGELOG `[Unreleased]`, log entry, once pimdir draft-03 is tagged.
