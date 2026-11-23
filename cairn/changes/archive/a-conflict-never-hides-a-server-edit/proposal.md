---
cairn: change
id: a-conflict-never-hides-a-server-edit
status: landed
created: 2026-10-01
---

# A conflict never hides a server edit

> Cross-repo change, same id in pimdir (the rule, vector 32, a new vector 33) and here (the engine). The pimdir side lands first: the vectors this engine reproduces are read from the sibling `../pimdir` checkout (tests/vectors_sync.rs `spec_dir`). The full rationale is pimdir/cairn/changes/a-conflict-never-hides-a-server-edit/proposal.md.

## Why

CI on master went red on 2026-10-01 (run 36866646724, commit ca8059d, a lockfile-only bump: proptest and rand unchanged). `conflict_property::conflict_interleavings_are_reported_resolved_or_kept` drew a case where a source's server holds a body the shared item does not, with nothing conflicted and nothing reported (the law at tests/engine/conflict_property.rs, "silently diverges"). It is a latent data loss, two rules combining:

1. **`reconcile_content` drops a server edit under an item conflict** (src/sync.rs, the `if local.status == PimdirStatus::Conflict` branch). A placement projecting the item's cross-source conflict has no `conflict_revision`; the branch returns `Untouched` whatever the remote revision says. The delta lists the member once, so the edit is never seen again. The requirement "An item-level conflict is held, a binding conflict is reconciled" (cairn/spec/sync.md) states this behaviour on purpose; it is what changes.
2. **`Remove` on a conflicted binding writes a false base** (src/mutate.rs, `PimdirMutation::Remove` in `writes`). It sets `base.revision = conflict_revision` and drops `conflict_object`, keeping the old `base.object`. When another source's edit later revives the item, this source reads its server as in sync. `Edit` adopts both halves; `Remove` must too.

Reproduction, already done once (2026-10-01): the CI seed is appended to tests/proptest-regressions/conflict_property.txt (uncommitted), so `PROPTEST_CASES=0 cargo test --features cli --test engine conflict_interleavings` fails deterministically. Locally that seed regenerates a 19-op case shrinking to the same 9-op shape as CI's. Both, as fixed sequences for `check_conflict_model`:

- CI: `Edit(8089635574607175413, 5840897801265340469, 0), Sync(2252840766273600329), Edit(708511662345375018, 2425387062199730659, 1), ServerEdit(817242067714251277, 9561676652060422255, 0), Remove(0, 1187225718968202926), Sync(187819748482576969), Sync(320385055380388116), ServerEdit(1486278711845847894, 0, 0), Remove(0, 0)`. Fixed by rule 1 alone.
- Local: `Edit(16838380586123230907, 7364332625439662501, 234), Sync(15912438296197527623), Edit(15977273743644341664, 4044429169570956081, 238), ServerEdit(5805560717581819975, 15643808883992181873, 202), Remove(8941018803540869082, 1878029773367891722), Sync(8835900674930711871), Sync(11578306373923721664), ServerEdit(15408778193444608560, 3267151502060249558, 139), Remove(2569513160168011899, 4138144611257170816)`. Needs rule 2 as well.

## What

Both fixes were prototyped on 2026-10-01 and then reverted, pending this review:

```rust
// src/sync.rs, reconcile_content, inside `if local.status == PimdirStatus::Conflict`,
// after the existing "newer than conflict_revision" block:
if local.conflict_revision.is_none()
    && item.revision.is_some()
    && item.revision != base.revision
{
    return self.mark_conflict(local, item);
}
```

```rust
// src/mutate.rs, PimdirMutation::Remove in writes():
if let Some(revision) = source.conflict_revision.take() {
    let settled = source.conflict_object.take();
    let base = source.base.get_or_insert_with(|| PimdirBase { .. });
    base.revision = Some(revision);
    base.object = settled;
}
```

With both changes the whole suite passed, along with every property test at 1000 cases and the conflict property at 3000. The only exception was vector 32, whose expected `base_object` moves from `c1` to `null` in the pimdir change. Rule 1 applies regardless of `PimdirConflictPolicy`: the item's conflict is still open, so the source's own divergence is recorded beside it, not pulled or pushed into it.

Then a release: the engine's behaviour changes on a conflict path, the store format does not, so a patch release (0.5.x) and a neverest bump.
