---
cairn: log
change: an-automatic-settle-keeps-what-it-discards
date: 2026-10-30
---

# An automatic settle keeps what it discards

pimdir's rule of the same name (SYNC §7, §9, STORAGE §11; vectors 100 to 103).

## What landed

- src/mutate.rs: `PimdirMutation::Edit { .., overrule: bool }`; an overruling edit stages `PimdirWriteOp::Overrule { collection, handle }` before its upsert.
- src/change.rs: `PimdirWriteOp::Overrule`.
- src/hub.rs: the overruled handle's binding `conflict_object`, then the item's, each once and never the body adopted, kept as `PimdirOverruled` before the upsert applies; an overruling edit restating the shared body clears the item's conflict.
- src/client/write.rs: `refuse_unread_overrule`, `PimdirError::Unread` (`conflict-unread`) for a binding or item conflict holding no diverging body; a kept body's summary derived from its blob when the hub holds none.
- tests/vectors_sync.rs reads `overrule`; every `Edit` literal in the crate and its tests names `overrule: false`.
- Failing first: vectors 100 and 102 ("events differ": no `Overruled`), 101 ("the run failing differs"); tests/overrule.rs `an_overruling_edit_over_an_unread_item_conflict_is_refused` (with the check removed: applied). 103 passed before and after.
- Capabilities moved: mutate ("An overruling edit keeps what it discards", added).

## Left open

A person's edit (no `overrule`) restating the shared body over an item conflict leaves the item conflicted, as before: the hub cannot tell it from a source merely restating the shared body, which SYNC §9 says settles nothing.

## Migration

- Every `PimdirMutation::Edit` literal names `overrule` (`false` for a person's resolution and a queued `update`); neverest's `keep_store_bodies` (src/offline/driver.rs:4050) states `overrule: true` and fetches a placement's diverging body first, mapping `conflict-unread` to that fetch.
- An exhaustive `match` on `PimdirWriteOp` gains the `Overrule` arm (a connector or storage of its own ignores it unless it keeps losers; io-pimdir's write is the reference).
