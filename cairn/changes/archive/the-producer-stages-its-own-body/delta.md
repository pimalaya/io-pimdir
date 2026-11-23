---
cairn: delta
change: the-producer-stages-its-own-body
---

## MODIFIED Requirements

### Requirement: Blobs are content-addressed and sharded
An object's bytes SHALL live at `objects/<hash[0:2]>/<hash[2:4]>/<hash>`, immutable once written, so an identical body delivered twice is stored once. `PimdirBlobs` reads a blob independently of the SQLite connection, so a body can be read while the store is mutably borrowed to service a sync. It SHALL be read-only: `get`, `reader`, `path`, `files` and the hash, with no writer, so a handle that holds no lock (a reader's, or one opened from a path) cannot put a body in the tree.

A body SHALL be written only through `PimdirBlobStage`, which a producer (`PimdirProducer::stage_body`, `blob_writer`) or an owner (`PimdirStore::blob_stage`) hands out and which holds that handle's lock for as long as it lives: a producer's shared staging lock, an owner's owner lock (STORAGE §8). A body written through it is therefore never in the window a collector runs in.

### Requirement: A body may be ingested and emitted by streaming
The store SHALL be able to persist an object from a byte stream (`PimdirBlobWriter`, `Write`), computing its content hash incrementally under the store's algorithm, with the same temp, fsync, rename durability as a buffered write, so a large body is never held whole: `commit` answers the `PimdirObject` (hash and size) the bytes written name, so a body is never filed under a hash its caller computed apart. It SHALL expose a stored object as a readable stream for the same reason on the read side.

## ADDED Requirements

### Requirement: A producer stages its own body
`PimdirProducer::stage_body(bytes)` SHALL hash a body under the store's algorithm and write it to the blob tree while the producer's staging lock is held, answering the `PimdirObject` `enqueue` takes; `PimdirProducer::blob_writer()` SHALL do the same for a streamed body. A collector running between the stage and the enqueue SHALL be refused (`PimdirError::Staging`), so the body reaches the row that pins it.

#### Scenario: A collector between a stage and its enqueue
- GIVEN a producer that staged a body and has not enqueued yet
- WHEN the owner runs the collector, then the producer enqueues an `add` naming the body
- THEN the collector is refused with `Staging`, and the drain applies the add over the body (tests/staging.rs `a_body_staged_before_its_enqueue_survives_a_collector`)

### Requirement: A create refuses a link id live or pending
`PimdirProducer::enqueue_add(collection, link_id, flags, object, supersedes)` SHALL enqueue an `add` naming `link_id`, answering the queue row id, and SHALL refuse it with `PimdirError::Duplicate`, nothing appended, when the collection holds a live item under that link id (`live_item_for_link`, the check the drain parks on) that no pending `remove` or `move` withdraws, or when a pending `add` in the collection names it. The check and the append SHALL be one transaction, so two producers cannot both pass it.

#### Scenario: A create racing the same key
- GIVEN a collection with a live item `mid:a` and a pending add `mid:b`
- WHEN a producer adds `mid:a`, then `mid:b`, then `mid:c`
- THEN the first two are `Duplicate` (`Live(seq)`, `Pending(row)`) and leave the queue as it was, the third answers its row id (tests/staging.rs `a_create_naming_a_live_or_pending_link_id_is_refused`)

### Requirement: The submit intent is typed
`intent::PimdirSubmit` SHALL build and read the `submit` intent payload (pimdir STORAGE Annex B.2): `v: 1`, the optional performing `source`, the message's `object` (the body the row pins), the envelope `from` (empty for the null path) and `rcpts` (at least one), the optional `subject` and the optional `copy` collection. `to_action` SHALL validate and answer the `PimdirAction::Unknown` carrying it, its `object_hash` set; `from_payload` SHALL read a payload strictly, refusing an unknown version, a missing field or an empty `rcpts` with `PimdirActionError`.
