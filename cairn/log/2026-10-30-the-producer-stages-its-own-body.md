---
cairn: log
change: the-producer-stages-its-own-body
date: 2026-10-30
---

# The producer stages its own body

Three of four frontends wrote a body through `PimdirReader::blobs()` before opening their producer, outside the staging lock, so a collector running in between unlinked it and the drain retried an add whose body was gone (report-io-pimdir.md, blocker 3). The write side of the blob directory now exists only on handles holding a lock, and the producer stages, refuses duplicate creates and types the `submit` intent itself.

## What landed

- src/client/blobs.rs: `PimdirBlobs` is read-only (`get`, `reader`, `path`, `files`, the hash); `PimdirBlobStage` is the write side (`stage_body`, `writer`), holding its handle's lock (`Arc<PimdirLock>`) while it or a writer it opened lives; `PimdirBlobWriter::commit()` hashes the bytes written and answers the `PimdirObject`.
- src/client/producer.rs: `PimdirProducer::stage_body(bytes) -> Result<PimdirObject>`, `blob_writer() -> Result<PimdirBlobWriter>`, `enqueue_add(collection, &link_id, flags, object, supersedes) -> Result<i64>` refusing a link id live (`live_item_for_link`, not withdrawn by a pending `remove` or `move`) or named by a pending `add` as `PimdirError::Duplicate { collection, link_id, existing: PimdirDuplicate::{Live(seq), Pending(row)} }`, checked in the enqueue transaction.
- src/client.rs: `PimdirStore::blob_stage()`, the owner's write side for a connector streaming fetched bodies.
- src/intent.rs: `PimdirSubmit { source, object, from, rcpts, subject, copy }` with `validate`, `to_action` and a strict `from_payload` (`object` and a non-empty `rcpts` required).
- Tests: tests/staging.rs `a_body_staged_before_its_enqueue_survives_a_collector` (its reader-path form failed before the change: the drain applied 0 of 1 adds, the body collected), `a_streamed_body_holds_the_lock_until_its_commit`, `a_create_naming_a_live_or_pending_link_id_is_refused` (fails with the check disabled); src/intent.rs submit tests over the payload himalaya writes and the ones it refuses; a compile_fail doctest on `PimdirBlobs` (`reader.blobs().writer()`).
- Capabilities moved: store ("Blobs are content-addressed and sharded", "A body may be ingested and emitted by streaming", modified; "A producer stages its own body", "A create refuses a link id live or pending", "The submit intent is typed", added).

## Migration for the frontends

- himalaya src/pimdir/backend.rs `write_blob` (:1200): `producer.stage_body(raw)?` in place of the hand-written writer; the `add` at :586 through `producer.enqueue_add(&collection, &link_id, flags, Some(&object), None)`, mapping `Duplicate` to its "already exists" answer; `SubmitPayload` (:951-964) replaced by `intent::PimdirSubmit { source, object: object.hash, from, rcpts, subject, copy }.to_action()?`; src/pimdir/client.rs:71 keeps a read-only `PimdirBlobs`.
- cardamum src/pimdir/backend.rs `stage` (:469-482): `producer.stage_body(contents)?`, the `add` (:226) through `enqueue_add`; the client's `blobs` field (src/pimdir/client.rs:24, :50) stays read-only.
- calendula src/pimdir/backend.rs `stage` (:340-357): open the producer, `stage_body`, then `enqueue_add` for the create (:293), `enqueue` for the update; the gate's `Unsupported` matches keep working.
- unfog src/pimdir/backend.rs `stage` (:700-714): the same, `enqueue_add` carrying `supersedes` for a cross-collection move (:404); tests reading `reader.blobs()` are unchanged.
- neverest: the owner's streaming connectors take `PimdirBlobStage` (`store.blob_stage()`) instead of `PimdirBlobs` (src/offline/remote.rs:32, :78, :308, :336, `HydrateSink` :1574-1597 drops its own hasher, `commit()` answering the object; test at :1928); the conflict resolutions (src/conflict.rs:176-197, src/offline/driver.rs:4370-4385) stage through the producer they already open (`producer.stage_body(body)`) or `store.blob_stage()`; src/offline/submit.rs reads the intent with `PimdirSubmit::from_payload` in place of `SubmitMeta` (:150-169), its tests' payloads gaining `object`, and `prepare` keeps a read-only `PimdirBlobs`.
