---
cairn: change
id: gtd-hardening
status: landed
created: 2026-10-28
---

# Read older task summaries, own a store without rebuilding it, supersede safely, know where tasks go

## Why

Track A of unfog cairn/changes/gtd-hardening (H1, H4, M6), following pimdir cairn/changes/gtd-hardening.

- **H1.** neverest and calendula lock io-pimdir 3b4bed8, whose owner reconciles `task_summary` to a shape without `priority`, `categories`, `parent` and `mid`. A reader at HEAD names them unprobed, so unfog's `gtd inbox`, `task list` and `task start` fail "no such column: s.priority" (reproduced). STORAGE §6 asks a reader to read a missing column as `NULL`, as this crate already does for `collections.role` and `sources.covered_at`. And unfog's local time strategy opens the store as owner for one write, which reconciles (rebuilds) a store another owner keeps, possibly a whole mailbox.
- **H4.** A GTD move is an add plus a remove staged together; a parked add or a refused push still lets the remove through. pimdir now states an `add` carrying `supersedes`, applied with its tombstone in one transaction, whose delete waits for the add's push.
- **M6.** pimdir now states `x-vtodo`, which this crate must gate on and expose.

## What

- The reader and the query client probe the task columns (tests/older_task_summary.rs, failing before the fix with the reproduced error).
- `PimdirStore::open_current`, an owner open that refuses anything `open` would create, migrate or reconcile (`PimdirError::NotCurrent`), asked by unfog's track B.
- `PimdirAction::Add::supersedes` through the codec, the producer's gate, the drain, the reader's overlay (the superseded item hidden while the add is pending) and the sync engine's hold (`PimdirLoaded::held`), with tests for a parked add and a refused push.
- `capability::X_VTODO`, the gate on it, `PimdirReader::accepts_vtodo`.
- `PimdirReader::replied`, over pimdir's `list_replied`, for unfog's review (umbrella M8).
- spec/ re-vendored, vectors/sync 71 and 72 run.

## Out

- Declaring `x-vtodo` from a CalDAV server's `supported-calendar-component-set`: the connector's (neverest).
- The clients' use of `supersedes` (unfog track D, Android track F).
