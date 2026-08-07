---
cairn: log
date: 2026-08-07
change: persist-binding-conflict
---

# The store persists a per-source content conflict

io-replica's hub had just been fixed to round-trip a per-source content conflict
on `ReplicaSourceBinding` (`conflicted`, `conflict_revision`), but `bindings`
had no column for either, so the state died at the storage boundary: kept in
memory, dropped on write, read back as `Dirty` by the next process. Across a
restart the upstream fix bought nothing — the sync re-derived the push its
remote had already rejected, every run, without converging.

`bindings` now carries both columns, through `LOAD_BINDINGS` /
`INSERT_BINDING` / `UPDATE_BINDING` and `binding_from_row`, with the revision
meaningful only while conflicted (spec §11). The schema is byte-identical to
`pimdir/migrations/0001_init.sql`, which gained the same columns.

Folded into **version 1** rather than added as version 2, the spec being still
`draft`; `sql::VERSION` stays `1`. That leaves an earlier-draft store stamped
current yet missing the columns, which the version check cannot detect, so
`init_schema` now runs `reconcile_draft_shape`: any folded-in column found
missing is added with `ALTER TABLE … ADD COLUMN`, guarded by `PRAGMA table_info`
so it is a no-op for every store after the first open. Spec §6 requires exactly
this (reconcile or refuse) for a draft store.

Capabilities moved: **store**.

Next: release io-replica and io-pimdir, bump neverest's dependencies, and flip
its canary assertion — verified locally to fire with both working trees patched
in.
