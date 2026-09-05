---
cairn: change
id: engine-merge
status: landed
created: 2026-09-03
---

# Fold io-replica into io-pimdir, and the store onto the 2026-09-03 spec

## Why

The pimdir standard became three parts on 2026-09-03: storage, sync and search. io-pimdir implemented the storage part against the spec as it stood before that day, and io-replica implemented the sync part behind a storage trait io-pimdir was the only implementor of. Two crates, one seam, and a spec each now diverged from: the store lacked the typed summaries, the probes table, the change feed and the refcount floor, and the engine ferried an opaque summary the spec no longer has.

## What

One crate. The engine's coroutines move into io-pimdir's no_std core under the `Pimdir` prefix, the `ReplicaStorage` trait goes (the store services its own yields, a foreign driver calls `service`), and the store catches up with the spec: five summary tables and `item_address` derived under Annex A, probes as rows, the change feed and its triggers, the refcount floor, the canonical statements verbatim. Four rules the spec and the hub disagreed on settle on the spec's side: retention in the core, an unlinked upsert folding into its binding, a created placement carrying its origin, a KeepBoth fork under a minted key. No migration: a store from an earlier draft is refused and recreated.
