---
cairn: change
id: a-producer-proves-it-can-write
status: landed
created: 2026-10-30
---

# A producer proves it can write

## Why

Since the N8 change of 2026-10-30 (store-audit-2026-10-30) a producer takes the staging lock at its first staged body, not at its open. An app's `account check` wants to prove the store is writable by its producer before the first real write, so cardamum's check opens a blob writer and drops it, which writes a temporary file into objects/ (and removes it) only to prove the lock is free; and a collector holding the lock makes that check wait rather than answer.

## What

- `PimdirProducer::probe(&self) -> Result<(), PimdirError>`: takes the staging lock shared without waiting and releases it, and takes the database's write lock (`BEGIN IMMEDIATE`) and rolls it back, writing nothing into the store. A collector holding the staging lock is `PimdirError::Busy`; a database it cannot write is the error SQLite answers. A producer whose staging window is open holds the lock already and probes the database alone.
- Migration: cardamum's `check` calls `producer.probe()` in place of opening and dropping a blob writer; the other apps' checks may do the same.
