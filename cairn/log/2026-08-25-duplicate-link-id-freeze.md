---
cairn: log
change: duplicate-link-id-freeze
date: 2026-08-25
---

# The store persists an identity a collection holds twice

The io-pimdir half of the cross-repo change. io-replica detects the duplicate and refuses to derive for it; this makes the fact survive, which is the part that matters: the second copy appears in exactly one enumeration, and an incremental one never mentions it again, so a freeze that is not persisted forgets on the next run and the item goes back to being deletable.

This is also where the evidence used to be destroyed. `UPDATE_BINDING` set `handle = :handle` on the existing `(collection, link_id, source)`, so the second copy silently repointed the binding and no layer above could afterwards tell that the source held the identity twice.

## What landed

- **`bindings.ambiguous_handles`** (capability `store`), a nullable JSON array, folded into version 1 and reconciled on open beside the other folded-in columns. `write` persists it, `load` returns it on `ReplicaSourceBinding`, and it round-trips a reopen.

- **`UPDATE_BINDING` no longer carries `handle`.** A binding pins one handle; rebinding legitimately, after a handle-space change, goes through the rebuild that drops the old spine and inserts the new one, never through an in-place update. The diff that calls it keeps the bound handle and records the incoming one instead, so a store written by an older engine, or a consumer staging its own writes, cannot repoint one either. The canonical statement moved with it (pimdir `update_binding`, now stated explicitly rather than left implicit).

- **`pimdir check` reports ambiguous bindings**, read-only. Not a defect: two copies of one message is redundancy, and the store records it rather than judging it. It is reported because it is the reason those items stop syncing, and an operator looking at a frozen item has no other way to see why.

- **`ENSURE_INDEXES`** replaces the three per-index constants and runs on every open rather than only when a column is missing. That was already wrong for the indexes this audit added: they index columns that were always there, so no missing column would have triggered them, and a store that kept the old query plans would have kept scanning where the schema says it seeks.

## Adapting to the engine

This crate pinned io-replica from crates.io, so the `engine-algorithm-audit` changes were invisible to it until now. Taking the new types meant taking that adaptation too: `load` gained a scope (a handle scope resolves through `LINK_FOR_HANDLE` first, since the hub is keyed by link id), `DropPlacement` gained a reason, and `ReplicaCollection` is gone. The dependency is a path dep until io-replica is published, the pattern this org already uses for an extracted-but-unpublished lib.

The `Superseded` drop reason is the one that matters for data here: without it a rekey's drop-then-upsert read as a mass delete through the hub.

## Verification

- 81 tests green, `cargo clippy --all-targets --all-features` clean, `cargo fmt`.
- `tests/duplicate_link_id.rs`: the handles survive a reopen; a write never repoints a binding and records the incoming handle instead; an ordinary write clears nothing; the engine clearing the freeze clears the column.
- The draft-reconcile suite covers the new column and all five new indexes, so a store written by an earlier draft reconciles on open.
- The spec-fidelity suite compares the inlined DDL against `pimdir/migrations/0001_init.sql` through SQLite's own pragmas and every canonical statement name against the constants, so the column and the statement changes are checked against the format on both axes.

Capabilities moved: `store`.
