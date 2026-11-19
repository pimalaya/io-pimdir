---
cairn: log
change: a-runner-of-its-own-counts-its-verb
landed: 2026-11-19
---

# A runner of its own counts its verb

Audit line N2: the in-flight count `run` holds is public. `PimdirSourceStore::running(&self) -> PimdirRunning` raises it under the writer lock held shared and the guard drops it; `client::PimdirRunning` now owns an `Arc` of the lock instead of borrowing its shared state, so a runner can hold it while it services yields and hands the store to a pager mutably. `run` takes the guard through the method; `PimdirLock::running` is `pub(crate)`. Additive; nothing `run` did moves.

Capabilities moved: store ("A store never collects itself": the public guard, a scenario).

Tests: tests/gc.rs `a_running_guard_keeps_the_collector_out` (the collector on the holding handle and on another of the process, `evict_before` and `release_before` refuse as `InFlight`, the body kept, reclaimed once the guard drops). A new public surface, so against the old code it fails by not compiling; the behavioural fail-before is neverest's test of its drive loop (neverest cairn/log/2026-11-19-the-drive-loop-counts-its-verb.md).
