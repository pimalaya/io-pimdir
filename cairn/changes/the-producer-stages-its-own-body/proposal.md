---
cairn: change
id: the-producer-stages-its-own-body
status: landed
created: 2026-10-30
---

# The producer stages its own body

## Why

STORAGE §8 asks a producer to write a body inside the staging window it holds `objects.lock` shared for, so a collector cannot unlink it before the enqueue pins it. Three of four frontends (himalaya backend.rs:500 :558, calendula backend.rs:593, unfog backend.rs:396 :426) hash and write the body through `PimdirReader::blobs()` before opening the producer; calendula and unfog reproduced the loss, himalaya under strace. Only cardamum (backend.rs:416-447) gets it right. When three of four callers misuse an API, the API is wrong (report-io-pimdir.md, blocker 3).

The same audit found more that every frontend rebuilds by hand:

- a create whose link id is live or already pending is enqueued, reported created, then parked by the owner (cardamum, calendula), and three apps return a link id no verb accepts;
- the `submit` intent payload is hand-rolled by himalaya (backend.rs:868-882) and neverest (offline/submit.rs:150-169).

## What

- `PimdirProducer::stage_body(bytes) -> PimdirObject`: hashes with the store's algorithm and writes the blob under the producer's staging lock, answering the object `enqueue` takes. `PimdirProducer::blob_writer()` is its streaming variant, `PimdirBlobWriter` hashing as it writes.
- The reader's blob handle is read-only (`get`, `reader`, no writer), so a frontend cannot write a body outside the window; the write side is `PimdirBlobStage`, which an owner gets from `PimdirStore::blob_stage()`. **Breaking.**
- `PimdirProducer::enqueue_add`, refusing an `add` whose link id is live or pending in the collection (`live_item_for_link`, the queue) as `PimdirError::Duplicate`, answering the queue row id it enqueued otherwise.
- `PimdirSubmit`: the `submit` intent payload (Annex B.2) as a typed struct with its codec, used by the frontends and the performer.
- The owner-side drafts touching this surface (an-ambiguous-intent-awaits-confirmation, raw-payloads-print-their-length) stay undecided; this change does not presume them.
