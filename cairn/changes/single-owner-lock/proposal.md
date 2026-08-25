---
cairn: change
id: single-owner-lock
status: active
created: 2026-08-25
---

# One owner, enforced, and failing fast

## Why

SPEC.md §8 states the rule and nothing enforces it: *at most one process owns the database at a time, and owners SHOULD take an advisory lock on the store directory*. No implementation takes one. The only guard is SQLite's write lock behind a 30-second `busy_timeout`, which serialises statements without serialising *operations*: two owners can each read a consistent snapshot and then each act on it.

The operator CLI is the second owner in practice. `item restore` drains, `item purge` and `queue cancel` write, and any of them can run while a neverest daemon syncs the same store. The queue drain is already safe by construction (`CLAIM_ACTION` deletes the row it is about to apply, first, and skips when the delete returns nothing), but `purge`, `write_rekeyed` and `revive` are not.

Waiting is the wrong default. A daemon syncing a large mailbox holds its transaction long enough that a 30-second wait is neither a wait nor a failure, just a stall with no signal. A program on top can retry, back off, queue the intent, or tell the user, and it can only do that if it is told.

## What

- **An exclusive advisory lock on the store directory**, taken by every owner handle for its lifetime, released by the kernel when the process dies. `fs4` provides it portably; an `O_EXCL` lock file does not, since a crashed owner leaves a stale lock and the escape hatch turns fail-fast into fail-always.
- **Fail fast**: an owner that cannot take the lock returns `PimdirError::Owned` immediately, naming the store. No timeout, no retry loop. Retrial policy belongs to whatever is on top.
- **Producers take a shared lock** across their blob write and enqueue. A producer is not an owner and several may run at once, but the pair has to be atomic against a collector: between the blob landing and the queue row pinning it, the body is an orphan file.
- **`gc` takes the exclusive lock too** (see `manual-gc`), which is what lets that change drop its grace window: a collector cannot run while an owner holds the store, so nothing it sweeps can be mid-flight. This is the GC-root relationship, expressed as a lock rather than a timer.
- `busy_timeout` stays: producers and owners still contend at the SQLite layer, and that contention is genuinely worth waiting out.

## Scope / non-goals

- The lock is per store directory, not per collection: the rule §8 states is about the database.
- Read-only handles take nothing. Any number may run against a store an owner holds.
- No lease, no liveness ping, no stale-lock recovery: the kernel releasing on process death is the whole mechanism, and adding a timeout to it would reintroduce what this removes.
