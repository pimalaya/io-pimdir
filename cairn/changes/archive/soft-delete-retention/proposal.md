---
cairn: change
id: soft-delete-retention
status: landed
created: 2026-08-07
---

# Retention: a store never loses an item, and an owner skips what it cannot do

## Why

A pimdir store is the local truth of a mailbox or an address book, and for a
backup it is the *only* copy: once a remote expunges an item, nothing else
holds it. Today `save_hub_diff` hard-deletes the row the moment the last
binding vanishes, garbage collection unlinks the body, and the copy is gone
with no way back. That makes "a read-only remote side plus a local store" an
unsafe backup recipe, which is the recipe neverest sells.

io-replica settled the contract on 2026-07-28 (`cairn/spec/storage.md`):
`DropPlacement` is the retention decision point, a storage MAY retain the row
instead of deleting it, and hiding retained rows from `load` is safe because the
merge reconciles only what `load` returns. So the engine needs no change at all;
this implements a contract io-replica already publishes.

The second half is the queue. An unrecognised action kind parks the row today,
and parking means *permanently unappliable*. That is wrong as soon as one queue
carries both store mutations (any owner can apply them) and capability-bound
intents such as a mail submission (only the owner holding the SMTP channel can).
An owner meeting a kind it does not know, or knows but cannot perform, has to
skip the row and leave it pending for the owner that can.

## What (design)

**Schema, in place at version 1.** The spec is a draft and stores are recreated,
never migrated, so `VERSION` stays `1`. `items` gains, right after `deleted`:

- `retained_at TEXT`: the RFC 3339 instant the last binding vanished; non-NULL
  means retained (soft-deleted). One column carries both the flag and the purge
  clock.
- `retained_by TEXT`: the source whose removal retired the item, diagnostic.

plus a partial index `items_retained ON items(collection, retained_at) WHERE
retained_at IS NOT NULL`, so the retained set is scanned without touching the
live rows. `pimdir/migrations/0001_init.sql` takes the identical DDL, and
`reconcile_draft_shape` heals a store written by an earlier draft of version 1.

**Retire instead of delete.** `save_hub_diff`'s "items gone in `new`" branch
updates the row (`deleted = 1`, `retained_at = strftime(…,'now')`,
`retained_by = <source>`) keeping `object_hash`, and deletes the item's now
source-less bindings. The stamp comes from SQLite, so the crate stays clock-free
and the tests stay deterministic: what a caller parameterises is the purge
*cutoff*, not the stamp.

`LOAD_ITEMS` gains `AND retained_at IS NULL`, which is the whole reason this is
safe: the retained row is invisible to the sync seam, so no delta and no full
resync ever re-derives against it.

**A retained row pins its bodies.** The hub diff releases the item's object
references as the item leaves the hub, so retiring compensates `+1` on the row's
`object_hash` and `conflict_object`, exactly as a queue row pins a queued body.
Revive releases the pin (`-1`), purge releases it and lets the existing
`collect_garbage` + `remove_blob` path unlink the blob once the count reaches
zero. No second garbage collector.

**Revive.** A link id reappearing while a retained row holds the primary key
`(collection, link_id)` updates that row (clearing `retained_at` /
`retained_by`, adopting the new content, keeping its `seq`) instead of
conflicting on insert. One branch serves both a source-side resurrection and a
client `add`, which is what makes restore an `Add` over values already in the
store: no new action kind. The queue's duplicate-link-id park already exempts
retained rows, since it tests the hub, which no longer holds them.

**Purge is the only true delete**: `purge(collection, seq)` for one retained
item, `purge_retained_before(cutoff)` for a time-based sweep (strictly before
the cutoff, so the boundary instant is kept), both reporting the bytes actually
reclaimed. Plus the read surface a trash view needs: `list_retained`,
`count_retained`, `retained_bytes`.

**Queue: skip, and let an owner acknowledge.** An unknown kind decodes into
`PimdirAction::Unknown { kind, payload, object_hash }` instead of erroring, so
an owner can inspect it, perform it out of band and acknowledge it; genuinely
malformed payloads (not JSON, no `v`) still park. The drain skips such a row,
leaving it pending and continuing with later rows (`PimdirDrainReport.skipped`).
`drop_action(id)` deletes one row, pending or parked, releasing its object pin
in the same transaction: one verb for both "cancel a queued action" and
"acknowledge an intent I performed". `fail_action(id, error)` exposes the two
existing failure statements coherently (bump `attempts`, or park with an error).

## Scope / non-goals

- No `VERSION` bump and no migration runner: the draft allowance covers this.
- No sweep schedule here. *When* to reclaim is the owner's policy
  (neverest's `store.purge-after`); *whether* a delete is terminal must be
  identical for every opener, so retention is unconditional and not
  configurable.
- io-replica is untouched.
- `submit` and every other capability-bound intent stays undefined here: the
  format carries a kind and a versioned JSON payload, nothing more.
