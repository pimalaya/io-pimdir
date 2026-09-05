---
cairn: tasks
change: persist-binding-conflict
---

# Tasks

- [x] `sql`: `bindings.conflicted` / `bindings.conflict_revision` in the schema,
      carried through LOAD/INSERT/UPDATE_BINDING; schema kept byte-identical to
      `pimdir/migrations/0001_init.sql` (verified by diff).
- [x] `client`: bind and read both; drop the revision unless conflicted (§11).
- [x] `init_schema`: `reconcile_draft_shape` heals a store from an earlier
      draft of v1, guarded by `PRAGMA table_info` and idempotent.
- [x] Tests (`tests/conflict.rs`): a conflict survives a reopen; resolving it
      clears it on disk; an earlier-draft store is healed on open and the heal
      is idempotent.
- [x] fmt + clippy clean, whole suite green (34 tests).
- [x] CHANGELOG; fold `delta.md` into `cairn/spec/store.md`; log; land.
- [ ] **Release** io-replica and io-pimdir, then bump neverest's dependencies
      and flip its canary assertion in
      `a_body_edited_on_both_sides_is_left_conflicted_not_overwritten`.

The release task stays unchecked here: it is neverest's canary to flip, and io-replica's release became moot when `engine-merge` folded it into this crate.
