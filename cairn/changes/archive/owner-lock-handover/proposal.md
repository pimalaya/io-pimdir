---
cairn: change
id: owner-lock-handover
status: landed
created: 2026-08-25
---

# A process could refuse itself the store it had just released

## Why

`single-owner-lock` made the owner lock the process's rather than the handle's, registered per store directory and shared, which is right: a two-sided sync opens one handle per source and a multi-account owner one per account, and each of those processes is one owner.

The registry tracked a `Weak<PimdirLock>` and let each handle hold the locked `File`. Those two facts do not happen at the same time. A strong count reaches zero the moment the last handle is dropped; the description it named stays open until that drop *returns*. A handle taken in the window between finds no live entry, opens a second description, and `flock` refuses it against this process's own.

The result is `PimdirError::Owned`, naming a store nobody else holds. Nothing above can act on it: it is not a wait, not a real conflict, and reproduces on no schedule, so it surfaces as a sync that occasionally declines to start.

Reproduced: four threads taking and releasing the role fail within a few thousand iterations. The premise was checked separately first, since the whole thing rests on it: a second file description in one process does conflict on `flock`.

## What

The registry owns the description and counts the handles sharing it. Releasing decrements under the registry's own mutex and, at zero, removes the entry, which drops the `File` inside that same critical section. The release and the next acquisition are therefore one operation, and the only `own` that opens a file is one finding no entry at all.
