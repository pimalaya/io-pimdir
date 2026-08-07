---
cairn: tasks
change: soft-delete-retention
---

# Tasks

- [x] `sql`: `items.retained_at` / `items.retained_by` plus the partial
      `items_retained` index, in place at version 1; `LOAD_ITEMS` filtered on
      `retained_at IS NULL`; the retire, revive, retained-read and purge
      statements. Kept structurally identical to
      `pimdir/migrations/0001_init.sql`.
- [x] `client`: `save_hub_diff` retires instead of deleting (stamp from SQLite,
      `retained_by` threaded from the write's source), deletes the item's
      source-less bindings, and compensates the object pin.
- [x] `client`: `insert_item` revives a retained row (pin released, `seq` kept)
      instead of conflicting on the primary key.
- [x] `client`: `list_retained`, `count_retained`, `retained_bytes`, `purge`,
      `purge_retained_before` plus `PimdirRetainedItem` / `PimdirPurgeReport`.
- [x] `init_schema`: both columns folded into `reconcile_draft_shape`.
- [x] `codec`: `PimdirAction::Unknown`, decoded instead of `UnknownKind`;
      malformed payloads still park.
- [x] `client`: the drain skips what it cannot apply (`PimdirDrainReport.skipped`),
      plus `drop_action` and `fail_action`.
- [x] Tests (`tests/retention.rs`, `tests/queue.rs`, `tests/roundtrip.rs`):
      retain-not-delete, hidden from load and from the live page, quiescent
      delta *and* full resync against a real `ReplicaClient`, revive, purge and
      its blob unlink, the cutoff boundary, the two-side in-flight delete, the
      skipped unknown action, `drop_action`'s pin release.
- [x] fmt + clippy clean, whole suite green.
- [x] CHANGELOG; fold `delta.md` into `cairn/spec/store.md`; log; land.
- [ ] **Blocked, not mine**: `Cargo.toml` still resolves `io-replica = "0.2"`
      from crates.io, which predates the binding-conflict fields, so the crate
      does not build without a local patch. Release io-replica 0.2.x (or patch
      the path) before this ships.
