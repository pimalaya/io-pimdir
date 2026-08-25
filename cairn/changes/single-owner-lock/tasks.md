---
cairn: tasks
change: single-owner-lock
---

# Tasks

- [ ] Add `fs4` to io-pimdir, behind the `client` feature.
- [ ] A lock file in the store directory (beside `pimdir.db`), opened and exclusively locked when a source-bound or otherwise owning handle is constructed, held by the handle, released on drop.
- [ ] `PimdirError::Owned`, returned immediately when the lock is held, naming the store path. No wait, no retry.
- [ ] Read-only handles take no lock.
- [ ] `PimdirProducer` takes a shared lock across blob write and enqueue, so the pair is atomic against a collector.
- [ ] Keep `PRAGMA busy_timeout = 30000` and `PimdirError::Busy`: owner-versus-producer contention is a different layer and worth waiting out.
- [ ] Tests: a second owner fails immediately rather than waiting; a reader opens fine while an owner holds it; a producer and an owner coexist; the lock is released when the owning handle drops.
- [ ] SPEC.md §8: the advisory lock becomes MUST, with fail-fast stated as the behaviour and the reason (a stall with no signal is not a policy a store can pick for its callers).
- [ ] `cargo test`, `cargo clippy --all-targets --all-features`, `cargo fmt`.
- [ ] CHANGELOG under `### Added`; fold `delta.md` into `cairn/spec/store.md`; log entry; mark landed.
- [ ] Hand over to `manual-gc`, which takes this lock and drops its grace window because of it.
