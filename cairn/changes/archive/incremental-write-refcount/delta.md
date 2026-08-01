---
cairn: change
change: incremental-write-refcount
---

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
inside the transaction and their blob files unlinked only after commit. The
incremental refcount is cross-collection correct: a batch adjusts a hash's count
by this collection's change alone, leaving other collections' references counted.
A crash SHALL leave at worst an orphan blob, never a row without its body.
The transaction SHALL begin with `BEGIN IMMEDIATE`, taking the store's single
writer lock (SPEC §7) up front: under WAL readers never block, two concurrent
writers serialise on the busy timeout, and a writer that cannot acquire the lock
SHALL fail with a clear `PimdirError::Busy` rather than a raw SQL error or a
failure deep inside the batch. Coordinating who writes (one owning process, or a
front daemon fronting a UI and a sync) is a platform decision, not enforced here.
The write SHALL be O(changed rows), not O(collection size): an incremental sync
that changed a handful of items does not re-examine or rewrite the whole mailbox.
