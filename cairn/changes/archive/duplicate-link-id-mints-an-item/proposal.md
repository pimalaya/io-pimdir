---
cairn: change
id: duplicate-link-id-mints-an-item
status: landed
created: 2026-08-28
---

# Store the second copy, and stop recording it instead

> Cross-repo change, same id in eight repositories, in this order:
> **pimdir** (the rule) → **io-replica** (the mint, and the removal of the freeze) → **io-pimdir** (here: the column goes, the refusal stays) → **io-webdav** (the refusal is named) → **neverest** (the resource name, the push guard, the report) → **himalaya**, **cardamum**, **calendula** (stop assuming a link id is unique).
>
> This **supersedes `duplicate-link-id-freeze`** (landed 2026-08-25). Do io-replica first: this change removes the state that one introduced, and the column cannot go while the engine still writes it.

## Why

`duplicate-link-id-freeze` fixed the write that destroyed the evidence, and it was right about that: a binding pins one handle, and repointing it silently is how a delete of the bound copy came to remove the only copy on the other side. What it could not fix is that the second copy still has nowhere to live, which the proposal named as its own successor: "Holding both would mean bindings keyed 1:N, a larger model change, and the natural successor to this one."

The successor turns out to be smaller than that, and on the other axis. The second copy does not need a second binding on one item; it needs an item of its own, because that is what it is: a separate resource, separately addressable, separately deletable, and (verified on a Posteo calendar, 2026-08-28) not necessarily even the same event.

Two things pushed it from theory to defect:

- **A frozen identity stores nothing.** Of two events sharing a `UID`, the replica held one. The other was on the server and in no local row, and neither the report nor any reader could say what was missing.
- **The freeze does not survive an enumeration that is not incremental.** The column's justification is that "the second copy appears in exactly one enumeration". A DAV collection whose server implements no `sync-collection` is listed in full every run, so the copy came back on every sync, was fetched in full to resolve its identity, was refrozen, and left its body unreferenced. Four bodies and four orphan blobs per run on that account, indefinitely.

## What

- **`bindings.ambiguous_handles` goes.** Column, codec, `load` projection, `write` path, CLI field, `check` counter. Version 1 is still edited in place while the format is draft, so an existing store reconciles on open, through the table rebuild the schema already uses where `ALTER TABLE` cannot express the change.
- **The refusal stays, and loses its record.** A write resolving an existing `(collection, link_id, source)` to a different handle SHALL be refused with a typed error rather than applied, and rather than recorded. The floor is unchanged: no write repoints a binding. What changes is that the caller now has somewhere to put the second copy, so refusing is a complete answer.
- **§12's licence is unchanged in shape.** A batch dropping a handle with `ReplicaDropReason::Superseded` still lets the binding move, per handle, not per batch. Only the else-branch changes: it refuses instead of recording.
- **A minted key is an ordinary key.** The store persists whatever `link_id` the engine hands it, takes no position on its shape, never parses a prefix, and never re-canonicalises one. `seq` allocation, retention, revival, dedup and the reader surface all treat `dup:<hint>#<handle>` exactly as they treat a bare `Message-ID`.
- **`check` may count, and resolves nothing.** The ambiguity counter goes; an informational count of minted keys per collection is welcome in the read-only pass, on the same terms as every other diagnostic it prints.

## Scope / non-goals

- **No repair, no dedup verb, no survivor.** Unchanged from the superseded change, and now unchanged for a store that holds both copies.
- **No hint-keyed read.** `link_placements` keeps pairing by key, so a minted item is not paired with its bare twin. `object_placements` still pairs the byte-identical case. A `hint_placements` statement is the natural successor and is not part of this.
- **No migration of existing stores beyond the column drop.** A store that froze an identity holds one of the two copies; the next sync enumerates the source, finds the second, and mints it. No resync, no rebuild.
- **The Java implementation in pimalaya/android is a second implementation of this format**, not a consumer of this crate, and follows the pimdir change on its own schedule. It is listed here so the removal is not mistaken for local cleanup.
