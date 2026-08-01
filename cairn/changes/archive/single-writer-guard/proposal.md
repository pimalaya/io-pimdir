---
cairn: change
id: single-writer-guard
status: landed
created: 2026-08-01
---

# Single-writer guard (BEGIN IMMEDIATE + loud busy)

## Why

pimdir SPEC §7 is single-writer: one process/handle writes at a time. Now that
two writers realistically coexist — a sync (Neverest) and a client (a Himalaya
pimdir backend) over one store — the store must serialise them cleanly instead of
corrupting or failing obscurely (action plan M6). Reads are fine (WAL gives
lock-free readers); the risk is two concurrent write batches.

A default (`DEFERRED`) write transaction takes no lock at `BEGIN`; it upgrades to
the write lock on the first write, so a loser discovers the contention deep inside
the batch as an opaque `SQLITE_BUSY`.

## What

- Begin the `write` batch with `BEGIN IMMEDIATE` so the single writer lock is taken
  up front: under WAL, readers never block, two writers serialise on the existing
  `busy_timeout`, and a writer that still cannot get the lock fails fast.
- Surface a busy/locked failure (at begin or commit) as a dedicated, clear
  `PimdirError::Busy` ("another writer holds the write lock; retry once it
  releases"), not a raw SQL error.

## Scope / non-goals

- Same-process single-owner coordination (a front daemon owning the writer, or one
  process hosting both UI and sync) is a platform/deployment decision (SPEC §7),
  documented, not enforced here.
- No change to the read path (WAL readers), the schema, or the write semantics.
