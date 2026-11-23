---
cairn: change
id: the-default-enqueue-strips
status: landed
created: 2026-11-18
---

# The default enqueue strips instructions

## Why

pimdir audit-2026-11-12, line C2: STORAGE §18 has a producer remove every `X-PIMDIR-` property from a resource the user did not write as it stands, and the crate's stripping enqueue was the opt-in one (`enqueue_foreign_body`), which no consumer called. calendula's raw `item create` so queued an imported file whole and read its `X-PIMDIR-ONLINE-MEETING` as the user's ask. The owner had kept it as it was on 2026-11-13, the risk judged small; the draft-14 lines ask that the default strips, or that the spec says why not.

## What

- `PimdirProducer::enqueue_with_body` strips a calendar or address-book body before it hashes it (`summary::strip_instructions`); a body of another kind is queued whole.
- `PimdirProducer::enqueue_user_body` is the old `enqueue_with_body`, the body kept whole: for a resource the user wrote as it stands or whose `X-PIMDIR-` properties they asked for.
- `enqueue_foreign_body` is removed, being now the default. `stage_body` and `enqueue` take the body as given, the caller bound by §18.
- calendula's path writing an online meeting the user ticked moves to `enqueue_user_body`; its raw import stays on `enqueue_with_body` and is stripped.
