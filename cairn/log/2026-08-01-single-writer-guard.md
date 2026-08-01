---
cairn: log
change: single-writer-guard
landed: 2026-08-01
---

# Single-writer guard (BEGIN IMMEDIATE + loud busy)

Hardened concurrent writes now that a sync (Neverest) and a client (the Himalaya
pimdir backend) realistically share one store (action plan M6). The `write` batch
begins with `BEGIN IMMEDIATE` (`transaction_with_behavior(Immediate)`), taking the
single writer lock up front — under WAL readers stay lock-free, two writers
serialise on the existing `busy_timeout`, and a writer that still cannot get the
lock fails fast at `BEGIN` rather than deep inside the batch on a deferred upgrade.
A busy/locked failure (at begin or commit) is mapped to a dedicated
`PimdirError::Busy` with a clear "retry once it releases" message instead of a raw
SQL error.

Coordinating who writes (one owning process, or a front daemon fronting a UI and a
sync) stays a platform decision — documented in the spec, not enforced by the
store.

Verified: existing suite green (11 tests), fmt + clippy clean. (A busy-contention
unit test would be timing-dependent and flaky, so none was added — the behaviour
is the WAL + `IMMEDIATE` + timeout configuration.)

Spec updated: `store` (MODIFIED: the write-batch transaction now takes the single
writer lock up front and fails busy loudly).
