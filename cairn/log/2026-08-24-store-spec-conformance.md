---
cairn: log
date: 2026-08-24
change: store-spec-conformance
---

# Three gaps against the pimdir specification closed

An audit against the specification checked out beside this crate found three divergences, each invisible until a store written by one version met another.

A store created by 0.2.0 could not be read at all: `items.sort_key` and `items_by_sort` were folded into version 1 after that release, and the §6 draft reconciliation never learned about them, so every paged read failed with `no such column: sort_key`. Both are reconciled now, and the regression test derives the older store from the current schema by dropping each folded-in column, so the next fold is covered without rewriting the test.

The §4.2 stamp agreement was unimplemented: only `PRAGMA user_version` was read, so a half-applied schema change opened as a store at whichever version the pragma held. Both opens now compare it with `store_meta.version` and refuse a disagreement.

Flags now use the `NULL` the format reserves for "nobody has read these", which became expressible when io-replica gained `ReplicaFlags::Unknown`.

A 0.2.0 store also carries no `ON UPDATE CASCADE`, which no `ALTER TABLE` can add, so reconciliation cannot reach it. Both opens now refuse such a store outright, spec §6's other branch: recreating it costs a resync of a derived cache, where opening it anyway leaves a store that can never follow a collection rename and only says so at the moment one is attempted.

Capabilities moved: store.
