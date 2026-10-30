---
cairn: log
change: a-queued-edit-names-what-it-changes
date: 2026-10-30
---

# A queued edit names what it changes

The merge half beside the-agreed-body-stays-pinned: the queued `update`'s `if_match` (store side landed with the-store-follows-storage-draft-05, the engine's `Edit` with the-engine-follows-sync-draft-05) and `SetFlags` as add and remove sets (pimdir SYNC §4).

## What landed

- src/change.rs: `PimdirChangeKind::SetFlags { handle, add, remove }`, keyed `set-flags`, `add`, `remove`, each a known flag set; the vectors' key `892471aec94fc225` for 02.
- src/sync.rs: `flag_delta` derives both from the merge against the set the run listed, else the base; an unknown merge or two empty sets push nothing.
- src/remote.rs: the seam states deltas, the unguarded window, and the `stated` identity.
- tests/engine/common.rs: the fake remote applies the two sets.
- Failing first: vectors 02, 03, 09, 26, 27 and 42 ("push differs": a replacement set and its old key `33f66fc5223a52f4`).
- Capabilities moved: sync ("A flag push names what it adds and removes", added; "Every change carries an idempotency key", modified), seam ("The push window a source leaves unguarded", added).

## Migration for connectors

Every connector matching `PimdirChangeKind::SetFlags { flags, .. }` applies `add` and `remove` as deltas instead (IMAP `STORE +FLAGS`, `-FLAGS`; JMAP keyword patch; Graph categories and flags per element).
