---
cairn: log
change: owner-lock-handover
date: 2026-08-25
---

# A process could refuse itself the store it had just released

`single-owner-lock` made the owner lock the process's rather than the handle's, registered per store directory and shared. That is right, and the registry it used was not: it tracked a `Weak<PimdirLock>` while each handle held the locked `File`, and those two facts do not happen at the same time.

A strong count reaches zero the moment the last handle is dropped; the description it named stays open until that drop *returns*. A handle taken in between found no live entry, opened a second description, and `flock` refused it against this process's own, producing `PimdirError::Owned` naming a store nobody else held. Nothing above can act on that: it is not a wait, not a real conflict, and reproduces on no schedule, so it surfaces as a sync that occasionally declines to start.

## What landed

The registry owns the description and counts the handles sharing it. Releasing decrements under the registry's own mutex and, at zero, removes the entry, which drops the `File` inside that same critical section. The release and the next acquisition are therefore one operation, and the only `own` that opens a file is one finding no entry at all.

## Verification

The premise was checked first, since the whole thing rests on it: a second file description in one process does conflict on `flock`. Then the race itself, four threads taking and releasing the role over the raw registry, which failed within a few thousand iterations against the old code (`is owned by another process`, with no other process) and does not against the new one.

The regression test lives in `src/client/lock.rs` rather than in `tests/`, because the window is between two operations on the registry and a loop through `PimdirStore::open` spends too little time in it to reproduce: the integration-level version passed against the broken code every time it ran.

The four existing owner-lock properties are unchanged and green. 104 tests, `cargo clippy --all-targets --all-features` clean, `cargo fmt`.

pimdir SPEC §8 gained the rule this rests on, alongside the other half of the same reading: the process-wide lock excludes other processes and nothing inside its own.

Capabilities moved: `store`.
