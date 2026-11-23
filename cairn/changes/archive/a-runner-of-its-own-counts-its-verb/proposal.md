---
cairn: change
id: a-runner-of-its-own-counts-its-verb
status: landed
created: 2026-11-19
---

# A runner of its own counts its verb

## Why

Audit line N2 (pimdir cairn/changes/audit-2026-11-12/tasks.md): the drive loop exists twice. `PimdirSourceStore::run` counts its verb in flight on the lock registry, so `collect_garbage`, `evict_before` and `release_before` refuse as `InFlight` while it runs (STORAGE §5). neverest's paged loop (src/sync/mod.rs `run_verb_paged`) resumes the coroutine itself through `service`, and the count was `pub(crate)` in effect (`PimdirLock::running` sat in a private module), so the collector's refusal was blind to every neverest verb. Harmless today, as neverest collects only after its verbs, but a second sweep or an app runner (the run module names a JNI one) would collect between two chunks of a verb.

## What

- `PimdirSourceStore::running(&self) -> PimdirRunning`, public, raising the count under the writer lock held shared as `run` does; `run` takes it through this method.
- `client::PimdirRunning`, the guard, now owning an `Arc` of the lock rather than borrowing it, so the store stays usable as `&mut` while it is held (neverest's pager takes the store mutably between pages). Its `Drop` drops the count.
- `PimdirLock::running` becomes `pub(crate)`; no behaviour moves for `run`.
- neverest holds the guard across `run_verb_paged` (neverest's own change).
