---
cairn: log
change: a-conflict-never-hides-a-server-edit
date: 2026-10-01
---

# A conflict never hides a server edit

CI's conflict property (run 36866646724, seed `b0c220b4…`) found a source silently diverged from its server. Two engine rules combined, both following pimdir draft-01; pimdir draft-02 changes both, and this engine follows.

## What landed

- src/sync.rs `reconcile_content`: a `Conflict` placement with no `conflict_revision` (the item's conflict) meeting a revision its base does not hold calls `mark_conflict`, whatever `PimdirConflictPolicy` says. Before, it returned `Untouched` and the delta never listed the member again.
- src/mutate.rs `Remove`: a conflicted binding's base adopts `conflict_object` with `conflict_revision`, `None` when unfetched. Before, it kept the old body beside the new revision, and a revival by another source read in sync.
- Tests: `an_item_conflict_records_a_server_edit_whatever_the_policy` (sync), `a_remove_adopts_the_fetched_diverging_body` and `a_remove_never_pairs_the_old_body_with_the_new_revision` (mutate), the CI and the local shrunk sequences pinned in tests/engine/conflict_property.rs, the CI seed kept in its regressions file. vectors/sync/32 (`base_object: null`) and 33 reproduce.
- README and spec-fidelity name draft-02; spec/ is unchanged, the draft touching no SQL.

## Capabilities moved

- sync: "An item-level conflict is held, a binding conflict is reconciled" gains the server-edit rule and its scenario.
- mutate: the `Remove` bullet of the offline mutation vocabulary.

## Verification

`cargo test --features cli` green with vectors_sync against ../pimdir; `PROPTEST_CASES=1000 cargo test --release --features cli` green; clippy `-D warnings` clean.

Release owed: a 0.5.x patch (behaviour on a conflict path, no store format change), then neverest's `cargo update -p io-pimdir`.
