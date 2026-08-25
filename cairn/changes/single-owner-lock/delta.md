---
cairn: change
change: single-owner-lock
---

# Delta

## ADDED Requirements

### Requirement: One owner, enforced, and failing fast
A store SHALL have at most one owner process at a time, and every owning handle SHALL take an exclusive advisory lock on the store directory (SPEC §8) for its lifetime. The lock lives on the open file description, so the kernel releases it when the process dies: a crashed owner leaves a lock file that locks nothing, and no stale-lock recovery is needed or offered.

An owner that cannot take the lock SHALL fail immediately with `PimdirError::Owned`, naming the store. It SHALL NOT wait: a daemon syncing a large mailbox holds its transaction long enough that a bounded wait is neither a wait nor a failure, just a stall with no signal, and retrial policy (retry, back off, queue the intent, tell the user) belongs to the program on top.

The rule is about processes, so several handles of one process SHALL share one lock: opening one handle per source, or one per account, is one owner. A read-only handle SHALL take no lock, and any number of readers MAY run against a store an owner holds.

A producer is not an owner. It SHALL take a *shared* lock on the store directory for its handle's lifetime, so several producers run at once and none of them keeps the owner out. What that lock delimits is the window a collector must not run inside: a body is written to the blob tree before the queue row that pins it, and between the two it is a file nothing references.

`PRAGMA busy_timeout` and `PimdirError::Busy` are unchanged by this: owners and producers still contend at the SQLite layer, and that contention is worth waiting out.

## MODIFIED Requirements

### Requirement: A write batch is one transaction
Unchanged except for its closing sentence, which said coordinating who writes was a platform decision and not enforced here. It is enforced here now, and points at the advisory lock above.

### Requirement: Terminal operations take the owner role directly
Purge, queue cancellation and orphan-blob reclamation have no action kind and cannot be queued: they SHALL take the owner role directly, without naming a source, and fail if the role is unavailable. When another process owns the store, or another writer holds its write lock, the CLI SHALL report it as a plain sentence naming the likely cause (a running sync) and never as a raw SQL or debug error dump.

### Requirement: The write source is resolved before anything is enqueued
Unchanged in what it resolves. What follows the enqueue changes: `item restore` SHALL append its action first and take the owner role only to apply it, reporting the action as queued when another process owns the store rather than failing. The action is already in the queue at that point, and the owner that holds the store is the one that will drain it.

## REMOVED Requirements

None.
