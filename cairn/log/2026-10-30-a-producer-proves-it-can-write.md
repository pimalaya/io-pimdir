---
cairn: log
change: a-producer-proves-it-can-write
date: 2026-10-30
---

# A producer proves it can write

## What landed

- src/client/lock.rs `PimdirLock::try_stage`: the shared staging lock without waiting, `PimdirError::Busy` when a collector holds it.
- src/client/producer.rs `PimdirProducer::probe(&self) -> Result<(), PimdirError>`: takes and releases the staging lock (unless the producer's window holds it), then `BEGIN IMMEDIATE; ROLLBACK`, writing nothing into the store.
- Tests: tests/staging.rs `a_probe_takes_the_staging_lock_and_writes_nothing` (a new API: failed to compile before).
- Capabilities moved: store ("A producer proves it can write without writing", added).

## For the apps

cardamum's `check` (src/pimdir/backend.rs:85-97) calls `producer.probe()` in place of opening and dropping a blob writer; himalaya, calendula and unfog may do the same in their checks, mapping `store-busy` to a retry.
