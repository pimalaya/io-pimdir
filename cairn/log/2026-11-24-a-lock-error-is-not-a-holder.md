---
cairn: log
change: a-lock-error-is-not-a-holder
landed: 2026-11-24
---

# A lock error is not a holder

The owner lock (`PimdirLock::own`), the collector's staging lock (`collect`) and the producer's probe (`try_stage`) mapped every `fs4::TryLockError` to `Owned`, `Staging` or `Busy`, so an `flock` that failed for another reason (a file system refusing locks, an I/O error) told the user another process held the store. Only `WouldBlock` is a holder now; any other error is `PimdirError::Io` carrying its cause.

Seen on 2026-11-24 in himalaya's Windows release job: a fresh store's first `PimdirStore::open` inside the Nix build sandbox answered `Owned` with no other process around. The cause there stays unknown; the next occurrence names it.

No test pins it: no file system at hand refuses `flock` on a file it opened.

Spec: store.md, the owner lock paragraph, states that a lock failing for another reason than a holder is `Io`.
