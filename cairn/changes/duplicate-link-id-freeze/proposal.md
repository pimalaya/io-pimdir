---
cairn: change
id: duplicate-link-id-freeze
status: landed
created: 2026-08-25
---

# Persist an ambiguous identity, and stop repointing a binding silently

> Cross-repo change, same id in three repos, in this order:
> **io-replica** (the invariant, the detection, the rules) → **io-pimdir**
> (here: persistence, and the write that destroys the evidence) → **neverest**
> (report it, end-to-end test). Do io-replica first: this change persists the
> state that one introduces.

## Why

A collection may hold two items with one link id (two messages with the same
`Message-ID`: double delivery, a retried `APPEND`, a restore, a migration). The
model has room for one — `items` is keyed `(collection, link_id)` and `bindings`
`(collection, link_id, source)` — and this store currently resolves the excess
by overwriting:

```sql
UPDATE bindings SET handle = :handle, ... -- PK (collection, link_id, source)
```

A write that resolves the identity to a different handle silently **repoints the
binding** from the copy it held to the new one. No error, no signal, and the
fact that the source holds the identity twice is gone. Everything downstream
then reasons about one handle as if it were the identity, and the consequence is
not cosmetic: on a two-sided sync, deleting the copy that happened to be bound
propagates a delete that removes the **only** copy on the other side, and a
later full enumeration revives the retained row and re-uploads the message to
the side it was deleted from. Both were reproduced against two IMAP servers.

This store is where the evidence dies, so this store is where it has to survive.
It is also the only `ReplicaStorage` implementation the ecosystem has: neverest,
himalaya-android-m3, android, linux and pimgate all reach the engine through it,
so persisting the state here fixes it for all of them at once.

## What

- **Persist the ambiguity.** `bindings` gains `ambiguous_handles`, a JSON array
  of the handles a source holds for this identity beside the bound one, on the
  same terms as `conflicted` / `conflict_revision`: written by `write`, returned
  by `load`, and round-tripped through `ReplicaSourceBinding` so the engine's
  freeze survives a restart. `NULL` and `'[]'` both mean "none"; the column is
  folded into schema version 1 and reconciled on open, the format being a draft.
- **Stop the silent repoint.** A binding update that would change the handle of
  an existing `(collection, link_id, source)` is the collision, and the store
  SHALL NOT perform it as an ordinary update. It keeps the bound handle and
  records the incoming one as ambiguous instead, which is the same decision the
  engine makes one layer up, enforced where a write can reach the table without
  passing through an upgrade (a queued `add`, a consumer's own mutation).
- **Report it, resolve nothing**, which is what §"Multiplicity is reported,
  never resolved" already says about an identity occurring in several
  collections. This is the same fact one axis in: `link_placements` keeps
  returning one row per collection, and a consumer that wants the handles a
  source holds beyond the bound one reads them from the binding.

## Scope / non-goals

- **The second copy still has no row.** This change records that it exists, not
  the copy itself, so the store still holds one item per identity per collection
  and a consumer still mirrors zero of two while an identity is ambiguous.
  Holding both would mean bindings keyed 1:N, a larger model change, and the
  natural successor to this one.
- **No repair.** The store neither deletes a duplicate nor chooses a survivor.
- **No new statement in the CLI's destructive verbs.** `pimdir check` MAY report
  ambiguous bindings as part of its consistency pass; repairing them is out of
  scope here.
