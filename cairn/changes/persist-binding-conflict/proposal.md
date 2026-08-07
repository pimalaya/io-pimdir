---
cairn: change
id: persist-binding-conflict
status: landed
created: 2026-08-07
---

# Persist a per-source content conflict

## Why

io-replica's hub now round-trips a per-source content conflict on
`ReplicaSourceBinding` (`conflicted`, `conflict_revision`) — the state that
says "this source and its own remote diverged and the merge left it
unresolved". The `bindings` table has no column for either, so the state died
at the storage boundary: the hub kept it in memory, the store dropped it, and
the next process read the placement back as `Dirty`.

That is the whole point of the upstream fix, so without this it buys nothing
across a restart: the sync re-derives the push the remote already rejected on
every run and never converges, and a client cannot tell which items need a
human. Invisible to mail (immutable bodies never conflict), fatal for
CardDAV/CalDAV.

## What (design)

`bindings` gains two columns, matching the pimdir spec (§4.3, §10, §11):

- `conflicted INTEGER NOT NULL DEFAULT 0`
- `conflict_revision TEXT`

carried through `LOAD_BINDINGS` / `INSERT_BINDING` / `UPDATE_BINDING` and
`binding_from_row`. The revision is written and read as meaningful *only while
conflicted*, per §11, so a resolved binding cannot hand a stale revision to the
next sync even if the column somehow holds one.

**Folded into schema version 1, not added as version 2**, since the pimdir spec
is still `draft` and version 1 is not frozen. `sql::VERSION` stays `1`.

The cost is that a store written by an earlier draft of version 1 is not
detectably out of date: its `user_version` already matches, so the runner would
do nothing and the missing column would surface much later as a query error.
Spec §6's draft allowance requires an implementation to reconcile the shape on
open or refuse the store; `init_schema` now calls `reconcile_draft_shape`, which
adds any folded-in column it finds missing (`ALTER TABLE … ADD COLUMN`, guarded
by `PRAGMA table_info`, so it is a no-op for every store after the first open).

## Scope / non-goals

- No new schema version and no migration runner: that machinery arrives with
  the first frozen version, when the draft allowance disappears.
- Only nullable / defaulted columns can be folded in this way; a folded change
  that could not be added to a populated table would need a real migration
  even while draft.
- The item-level cross-source conflict (`items.conflicted`,
  `items.conflict_object`) is untouched — a different fact, already persisted.
