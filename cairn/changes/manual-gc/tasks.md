---
cairn: tasks
change: manual-gc
---

# Tasks

- [ ] Remove `collect_garbage` from all six call sites; `write`, `write_rekeyed`, `apply_queued`, `drop_action`, `purge`, `purge_retained_before` no longer sweep or unlink.
- [ ] `PimdirStore::collect_garbage()` as the public collector: refcount-zero rows plus their blobs, plus orphan blob files, returning counts and bytes. Requires the exclusive lock (source-less handle).
- [ ] `pimdir gc` CLI verb over it, with an `*Output` type (Display + Serialize + JsonSchema) and its `json_schema.rs` entry.
- [ ] `check --fix` stops reclaiming and starts repairing: recompute refcounts from the pointer columns (the `UNION ALL` form, see pimdir `recompute-refcounts-linear`), clear dangling rows. Drop `--grace` and the confirmation prompt from `check`; `gc` needs neither.
- [ ] `purge` and `purge_retained_before` report rows retired rather than bytes reclaimed; update their `*Output` types and schema entries.
- [ ] Tests: a batch that stores an object without attaching it keeps the body until `gc`; `gc` reclaims it once nothing references it; `gc` refuses while an owner holds the store; `check --fix` repairs a seeded refcount drift; purge no longer reports bytes.
- [ ] SPEC.md §5 (an unreferenced object MUST NOT be deleted by a write; a collector removes it) and §14 (steps 3 and 5 leave the write algorithm); state the collector and its lock.
- [ ] io-replica: relax `ReplicaWriteOp::StoreObject`'s doc, which currently says a paired `UpsertPlacement` lands in the same batch.
- [ ] `cargo test`, `cargo clippy --all-targets --all-features`, `cargo fmt`.
- [ ] CHANGELOG (`### Added` the verb, `### Changed` the purge output, `### Fixed` the streamed-body loss); fold `delta.md` into `cairn/spec/store.md` and `cairn/spec/cli.md`; log entry; mark landed.
