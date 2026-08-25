---
cairn: log
change: single-owner-lock
date: 2026-08-25
---

# One owner, enforced, and failing fast

SPEC §8 said a store has at most one owner and that owners SHOULD take an advisory lock; nothing took one. The only guard was SQLite's write lock behind a 30-second busy timeout, which serialises statements without serialising operations: two owners could each read a consistent snapshot and then each act on it, and the operator CLI is the second owner in practice. Capabilities `store` and `cli` moved.

## What landed

- **An exclusive advisory lock on the store directory** (capability `store`), taken by every owning handle and held for its lifetime, in `owner.lock` beside the database. The lock sits on the open file description, so the kernel releases it when the process dies: a crashed owner leaves a lock file that locks nothing, which is what an `O_EXCL` lock file cannot promise and why it is not one.

- **`PimdirError::Owned`, immediately.** No timeout, no retry loop. A wait long enough to outlast a sync's transaction is neither a wait nor a failure, and the program on top is the only layer that can choose between retrying, backing off, queueing the intent and telling the user.

- **The lock is the process's, not the handle's.** §8's rule is about processes and an `flock` is about file descriptions, so a per-handle lock would have had a two-sided sync deadlock against itself the moment it opened its second source — as would every multi-account owner, both of which this crate documents as the normal shape. The owner locks a process holds are registered per store directory and shared, and withdrawn when the last handle referencing one drops.

- **Producers take a shared lock** (`objects.lock`), for their handle's lifetime rather than for the enqueue: the body is written to the blob tree *before* the queue row that pins it, so the window a collector must not run inside starts before `enqueue` is called. A separate file from the owner lock, and deliberately: producers exist to append while the owner syncs, so an owner's exclusive lock must not exclude them. Nothing takes it exclusively yet; `manual-gc` is what will.

- **`item restore` queues rather than fails** (capability `cli`). It used to take the owner role before enqueueing, so that an unresolvable write source failed before anything was appended. Only the *source resolution* has to happen first, which is what does now: the action is appended, and the owner role is taken only to apply it. A restore issued while a sync runs reports `queued`, which is what the queue is for. `item purge` and `queue cancel` genuinely need the role and now say so in a sentence.

## Tests

tests/owner_lock.rs, four properties: a second owner is refused rather than made to wait, and the store comes back when that owner is gone; a reader opens while another process owns it; a producer appends while another process owns it; and one process owning a store twice is one owner, with the lock released only when the last handle drops. The other owner is simulated by locking the file from a description the registry knows nothing about, which is what another process is from this crate's side.

## Deviation

The task list said `fs4`, and `fs4` it is, but std grew the same locks in 1.89 and its inherent methods shadow the trait's, so the two calls name the trait explicitly. The dependency exists to keep the crate's 1.87 MSRV and goes when that moves.
