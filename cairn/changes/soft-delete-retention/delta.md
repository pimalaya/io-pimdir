---
cairn: change
change: soft-delete-retention
---

# Delta

## ADDED Requirements

### Requirement: An item is retained, never deleted, when its last binding goes
`items` SHALL carry `retained_at` (TEXT, RFC 3339, nullable) and `retained_by`
(TEXT, nullable). When a write batch leaves an item held by no source, the store
SHALL **retire** the row rather than delete it: `deleted` set, `retained_at`
stamped by SQLite (`strftime('%Y-%m-%dT%H:%M:%fZ','now')`, so the crate needs no
clock), `retained_by` set to the source whose removal retired it, and
`object_hash` kept. The item's now source-less bindings SHALL be deleted with it,
so a retained row carries `deleted = 1` and no binding at all: the persisted form
of a removal that has finished propagating.

`retained_at` records when the **last binding vanished**, not when a server
deleted the item (unknowable). A revive clears it, so restore-then-redelete
restarts the clock.

A retained row SHALL pin its bodies: retiring compensates the object references
the hub diff released, so `object_hash` and `conflict_object` keep their
refcount and garbage collection never sweeps a retained body. Revive and purge
release that pin.

### Requirement: Retained rows are hidden from the sync seam
`LOAD_ITEMS` SHALL exclude retained rows (`retained_at IS NULL`), so a retained
item is absent from `load_hub`, from `load` and from every projection. This is
io-replica's "hiding rows from load is safe": the merge reconciles only what
`load` returns, so a retained item is never re-derived, re-added or re-pushed,
on a delta sync or a full resync. It is likewise absent from the live client
read surface, which already filters `deleted`.

### Requirement: Purge is the only true delete
The store SHALL expose the retained set and the only operation that destroys
data:

- `list_retained(collection, after, limit)` SHALL return a keyset page of a
  collection's retained items (`seq > after`, ordered by `seq`, at most `limit`),
  each carrying its `seq`, `link_id`, flags, level, raw `meta`, object hash,
  object size, `retained_at` and `retained_by`.
- `count_retained(collection)` SHALL count a collection's retained items, and
  `retained_bytes()` SHALL total the distinct object sizes the store's retained
  items hold, an upper bound on what a purge reclaims (a body a live item also
  points at survives).
- `purge(collection, seq)` SHALL delete one **retained** row, reporting whether
  it existed; a live item is never purged through it.
- `purge_retained_before(cutoff)` SHALL delete every retained row across the
  store whose `retained_at` is **strictly before** the caller's RFC 3339 cutoff
  (an item retained exactly at the cutoff is kept), reporting the items removed
  and the bytes reclaimed.

Both purges SHALL release the retained row's object pin and let the existing
refcount and garbage collection path unlink the blob once nothing references it;
there is no second collector. Retention itself is unconditional: *whether* a
delete is terminal must be identical for every opener, so only *when* to reclaim
is the caller's policy.

### Requirement: A reappearing link id revives its retained row
An item inserted while a retained row holds the primary key
`(collection, link_id)` SHALL revive that row: `deleted`, `retained_at` and
`retained_by` cleared, the new content adopted, the message's `seq` kept (ids are never
reused). One branch serves both a source-side resurrection and a client-staged
`add`, so restoring a retained item needs no new action kind: `Add` over the
values the row still holds is a restore. A duplicate-link-id check on the apply
path SHALL exempt retained rows.

### Requirement: An owner skips the actions it cannot apply
An action kind the store does not recognise SHALL decode as an opaque action
(kind, raw payload, and the body hash the payload pins) rather than an error, so
one queue can carry store mutations any owner applies beside capability-bound
intents (a mail submission) only a specific owner can perform. Genuinely
malformed payloads (not JSON, no supported `v`) SHALL still park.

The drain SHALL **skip** a row it cannot apply: the row stays pending, is never
parked (parking means permanently unappliable), and never blocks later actions
in the same collection. The drain report SHALL count skips beside applies and
parks.

### Requirement: A queued action can be cancelled or acknowledged
`drop_action(id)` SHALL delete one queue row, pending or parked, releasing its
object pin in the same transaction, and report whether the row existed. It
serves both cancelling a queued action and acknowledging an intent an owner
performed out of band. `fail_action(id, error)` SHALL record a failed attempt:
`None` bumps `attempts` and leaves the row pending (transient), `Some(error)`
parks it (permanent). A collection's pending actions SHALL expose each row's
`id`, since callers act on rows by id.

## MODIFIED Requirements

### Requirement: A write batch is one transaction
`write` SHALL apply its `ReplicaWriteOp` batch as a single SQLite transaction:
object bytes are written to the blob file (temp → fsync → rename) before the row
that references them; placement upserts/drops fold into the collection's hub and
are saved **by diffing the loaded hub against the absorbed one, touching only the
items and bindings that changed** (never a whole-collection delete-and-reinsert);
**object refcounts are maintained incrementally, applying only the per-hash
difference between the hub's object references before and after the batch** (never
a global recompute); zero-refcount objects are collected, their rows dropped
inside the transaction and their blob files unlinked only after commit. An item
the batch leaves held by no source is **retained, not deleted**, and keeps its
object references pinned. The incremental refcount is cross-collection correct: a
batch adjusts a hash's count by this collection's change alone, leaving other
collections' references counted. The write SHALL be O(changed rows), not
O(collection size), so an incremental sync that changed a handful of items does
not rewrite the whole mailbox.
A crash SHALL leave at worst an orphan blob, never a row without its body.
The transaction SHALL begin with `BEGIN IMMEDIATE`, taking the store's single
writer lock (SPEC §7) up front: under WAL readers never block, concurrent writers
serialise on the busy timeout, and a writer that cannot acquire the lock within it
SHALL fail with a clear `PimdirError::Busy` rather than a raw SQL error or a
failure deep inside the batch. The busy timeout SHALL be generous enough (30s) to
let a single process fan work across several same-source handles — one per worker,
to overlap network while the writes serialise — without a burst of large writes
tripping `Busy`. Coordinating who writes (one owning process, or a front daemon
fronting a UI and a sync) is a platform decision, not enforced here.

### Requirement: Producers append, only the owner pops
The store SHALL support the pimdir action queue: any process may act as a
producer whose sole write is the single enqueue transaction (ensure_collection,
at most one object upsert pinning a pre-written blob, one queue insert). Only the
owner SHALL read-and-remove queue rows: each pending action is applied to items
and bindings and its row deleted in the same transaction, so application is
exactly-once and never partially visible. Failing actions accumulate `attempts`;
permanently failing actions are parked with `error` set, skipped without blocking
later actions, queryable, and never silently deleted. An action the owner cannot
apply at all (a kind it does not recognise, or one it recognises but lacks the
capability to perform) is **skipped and left pending**, never parked, so another
owner can perform it.

### Requirement: A store from an earlier draft of the current version is reconciled on open
While the pimdir spec is `draft`, a schema change MAY be folded into version 1
rather than added as a new version (spec §6). A store written by an earlier
draft is then stamped with the current `user_version` yet lacks the folded-in
columns, so the version check alone cannot detect it.

On open, the store SHALL reconcile its shape: every folded-in column found
missing SHALL be added (`ALTER TABLE … ADD COLUMN`, which requires the column to
be nullable or carry a constant default), guarded so the check is a no-op for an
up-to-date store, together with any index over a folded-in column. Failing a
later query on a missing column is not acceptable. This requirement lapses when
the spec leaves `draft` and versions are frozen.

## REMOVED Requirements

None.
