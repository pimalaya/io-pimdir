---
cairn: tasks
change: the-producer-stages-its-own-body
---

- [x] Test: a collector run between a reader-written blob and its enqueue unlinks it (the current misuse, shown), and `stage_body` then enqueue keeps it (tests/staging.rs; the reader-path version failed on 4c3353e plus wave 1, the drain applying 0 of 1)
- [x] src/client/producer.rs: `stage_body` and its streaming variant `blob_writer` under the staging lock
- [x] src/client/blobs.rs, src/client/reader.rs: the reader's handle read-only (`PimdirBlobs` without `writer`, the write side `PimdirBlobStage`; a compile_fail doctest)
- [x] Test: an `add` whose link id is live, or pending, is refused before anything is appended
- [x] src/client/producer.rs: the duplicate refusal (`enqueue_add`, `PimdirError::Duplicate`), the row id answered
- [x] `PimdirSubmit` and its codec (Annex B.2); test against the payloads himalaya and neverest write today
- [x] Spec: store.md; fold, log, CHANGELOG (breaking)
- [x] Tell the frontends (Phase A of the plan): migration notes in cairn/log/2026-10-30-the-producer-stages-its-own-body.md
