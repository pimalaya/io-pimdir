---
cairn: tasks
change: manual-gc
---

# Tasks

- [x] Remove `collect_garbage` from all six call sites; `write`, `write_rekeyed`, `apply_queued`, `drop_action`, `purge`, `purge_retained_before` no longer sweep or unlink.
- [x] `PimdirStore::collect_garbage()` as the public collector: refcount-zero rows plus their blobs, plus orphan blob files, returning counts and bytes. Requires the exclusive lock (source-less handle).
- [x] `pimdir gc` CLI verb over it, with a `GcOutput` (Display + Serialize). No `JsonSchema`, no `json_schema.rs`: this repository has neither, and every other `*Output` here derives `Serialize` alone.
- [x] `check --fix` stops reclaiming and starts repairing: recompute refcounts from the pointer columns (the canonical `recompute_refcounts`, now inlined and off the substituted list), clear the dangling **bindings**. `--grace` and `--yes` are gone from `check`. The other two dangling kinds stay reported: an item whose object row is missing is still the item, and a queue row whose body is missing is still an intent, so deleting either destroys data rather than repairing it.
- [x] `purge` and `purge_retained_before` report rows retired rather than bytes reclaimed; update their `*Output` types and schema entries.
- [x] Tests (tests/gc.rs, plus the rewritten sweep assertions in tests/queue.rs and tests/roundtrip.rs): a batch that stores an object without attaching it keeps the body until `gc`; `gc` reclaims it once nothing references it, orphan files included and temp files excluded; `gc` refuses while a **producer** stages (the owner half is tests/owner_lock.rs, where the owner open is what fails); `check --fix` repairs a seeded refcount drift and a seeded dangling binding; a purge retires rows and the collector reports the bytes.
- [x] SPEC.md §5 and §14, landed in the pimdir repository as the change `no-write-collects`: an unreferenced object is not a deleted one, the two sweeps become one collector holding both locks, the grace period goes, steps 3 and 5 leave `write`, `collect_garbage()` is named, and purge reports rows retired. It supersedes `orphan-blobs-are-swept-by-nobody`, which landed hours earlier and made the grace period normative on the reasoning that no writer held a lock; `owner-lock-must` had since made that false.
- [x] io-replica: relax `ReplicaWriteOp::StoreObject`'s doc, which currently says a paired `UpsertPlacement` lands in the same batch.
- [x] `cargo clippy --all-targets --all-features`, `cargo fmt`, and `cargo test` on every target but tests/retention.rs, which does not compile against the in-flight `ReplicaChange` / `ReplicaChangeKind` split in the io-replica working tree. Unrelated to this change; its own sweep assertions will need the same rewrite the queue and roundtrip ones got.
- [x] CHANGELOG (`### Added` the verb, `### Changed` the purge output, `### Fixed` the streamed-body loss); fold `delta.md` into `cairn/spec/store.md` and `cairn/spec/cli.md`; log entry; mark landed.
