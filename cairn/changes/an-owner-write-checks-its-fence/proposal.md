---
cairn: change
id: an-owner-write-checks-its-fence
status: draft
created: 2026-10-18
---

# An owner write checks its fence

## Why

Ownership is an advisory lock on owner.lock (pimdir STORAGE §8), taken once per process and shared by its handles (single-owner-lock, owner-lock-handover). The lock is checked when it is taken, never when a write commits. Two cases slip through:

- a mobile worker suspended mid-run and resumed after its lock was lost and taken by another process (a store on a network filesystem, where §8 already says advisory locks are unreliable and a lease SHOULD enforce a single instance, without saying how);
- a handle kept past a release by a bug in the caller, whose next write lands under another owner's feet.

Borrowed from an external engine's design: a monotonic fencing token is the write-admission gate itself, re-checked inside the write transaction, with a lease's expiry only a safety net.

## What

- `store_meta` carries an owner epoch, drawn up when a process takes the owner role.
- Every owner write transaction checks, inside the transaction, that the epoch is the one its process drew, and fails with a stale-owner error otherwise, before any row moves. A producer's enqueue is not an owner write and is not fenced.
- On a network filesystem, the epoch is what §8's lease enforces: a process whose lease lapsed finds the epoch moved and stops.

## Open questions

- The cost: one read of `store_meta` per write transaction, against a row already read on most paths.
- Whether pimdir STORAGE §8 makes the fence a MUST for every owner or a SHOULD for a store on a network filesystem only.
- How the Android store, which owns pimdir.db from Java, draws and checks the epoch.

## Decision brief (2026-11-19)

The owner's to decide; nothing is implemented. Already ruled around it: Q9 of pimdir's audit-2026-11-12 (2026-11-12, F16) put the owner epoch after v1 as a migration raising neither floor, and landed the one sentence for now (STORAGE §8: a lock file is never removed nor replaced while the store exists); the Android app takes no owner lock at all (owner decision, 2026-11-13).

### Options

- **(a) After v1, as a minor migration (recommended).** A `store_meta` column added by a storage-v1 minor (STORAGE §4.4: a statement names its columns, so an added column raises neither `read_version` nor `produce_version`), drawn when a process takes the owner role, checked inside every owner write transaction. Nothing frozen at v1 moves; the fence arrives when a store on a network filesystem is a supported case.
- **(b) Before v1, in the v1 schema.** The same column and two statements in storage-v1.0.0, a MUST for every owner. Holds the tags for a case no consumer runs today (every store is on a local disk), and makes the Android app, which takes no lock, either break the rule or draw an epoch from Java.
- **(c) Never.** Keep the advisory lock and the §8 sentence as the whole story. Leaves the suspended-worker and kept-handle cases to the lock alone.

### Cost per consumer

- pimdir: one `store_meta` column, a statement drawing the epoch and one checking it, a scenario in checks/invariants.sh, §4.3 and §8 text; under (a) a minor migration and its rehearsal.
- io-pimdir: draw in src/client/lock.rs when the role is taken, one read of `store_meta` inside each owner write transaction (src/client/write.rs and every other owner write), a `PimdirError` for a stale owner with its code, a two-process test.
- neverest, himalaya, cardamum, calendula, ortie: nothing but the io-pimdir upgrade and one more code in their error tables (neverest owns the store; the others are producers and readers, never fenced).
- Android: under (a) or (b), an exemption stated beside "The app takes no lock on its store" (one process, the store in its private directory), or a Java draw and check in every write of its own; under (c), nothing.

### Recommendation

(a): it is what Q9 already decided, and nothing in it gates storage-v1.0.0. This change stays draft until a store on a network filesystem, or a second owning process on one device, is a case the owner wants supported. Android stays exempt for the reason of 2026-11-13.
