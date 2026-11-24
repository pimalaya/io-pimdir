---
cairn: log
change: a-release-unlocks-before-closing
landed: 2026-11-24
---

# A release unlocks before closing

Found on the live night of 2026-11-24 (night 2), chasing the `Owned` himalaya's Windows release job hit on 2026-11-24 in `tests/pimdir_queue.rs` (`a_copy_into_a_mailbox_holding_the_message_lists_one_envelope`, a fresh store opened, closed, opened again).

Outside the sandbox the test binary failed 5 of 80 parallel runs with four test threads, 0 of 80 with one: the other thread spawns `himalaya` children, and a child forked while the store is open holds a copy of the locked open file description until its `exec`. `PimdirLock`'s release closed this process's descriptor alone, so the `flock` stayed on the description while the child lived, and the reopen read it as another holder (`WouldBlock`, reported `Owned` since 0.20.2 reports only that as a holder).

A release now unlocks the description (`FileExt::unlock`) before it closes, for the registry's owner lock and for a staging lock. An unlock failing is logged at debug and the close still runs.

Pinning test: `client::lock::tests::a_copy_of_the_lock_a_forked_child_holds_does_not_keep_the_store` (a `try_clone` of the registry's description stands in for the child's copy); it fails before the fix with "is owned by another process".

Spec: store.md, the owner lock paragraph, states that a release unlocks the description before closing it.

Not explained by this: carillon's SQLite `disk I/O error` (1802) in its macOS release job the same day.
