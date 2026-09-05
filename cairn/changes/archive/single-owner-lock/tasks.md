---
cairn: tasks
change: single-owner-lock
---

# Tasks

- [x] Add `fs4` to io-pimdir, behind the `client` feature.
- [x] `owner.lock` in the store directory (beside `pimdir.db`), exclusively locked by `PimdirStore::open`, held by the handle, released on drop. Registered per store directory and shared within the process: a per-handle lock would deadlock a two-sided sync against itself on its second source, and every multi-account owner likewise.
- [x] `PimdirError::Owned`, returned immediately when the lock is held, naming the store path. No wait, no retry.
- [x] Read-only handles take no lock.
- [x] `PimdirProducer` takes a shared lock on a second file, `objects.lock`, for its handle's lifetime: the blob write is the caller's and happens before `enqueue`, so the window cannot be taken inside it. A separate file from the owner's, or an owner's exclusive lock would shut out the producers the queue exists for. Nothing takes it exclusively until `manual-gc`.
- [x] Keep `PRAGMA busy_timeout = 30000` and `PimdirError::Busy`: owner-versus-producer contention is a different layer and worth waiting out.
- [x] Tests: a second owner fails immediately rather than waiting; a reader opens fine while an owner holds it; a producer and an owner coexist; the lock is released when the owning handle drops.
- [x] SPEC.md §8: the advisory lock becomes MUST, with fail-fast stated as the behaviour and the reason (a stall with no signal is not a policy a store can pick for its callers). Landed in the pimdir repository as the change `owner-lock-must`, which also names both lock files in §3 and records the Android store as non-conformant until it takes them.
- [x] `cargo clippy --all-targets --all-features`, `cargo fmt`, and `cargo test` on every test target but tests/retention.rs, which does not compile against the in-flight `ReplicaChange` / `ReplicaChangeKind` split in the io-replica working tree. Unrelated to this change and untouched by it.
- [x] CHANGELOG under `### Added`; fold `delta.md` into `cairn/spec/store.md`; log entry; mark landed.
- [x] Hand over to `manual-gc`, which takes this lock and drops its grace window because of it.
