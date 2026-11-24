---
cairn: log
change: a-copy-revives-its-sources-own-tombstone
landed: 2026-11-24
---

# A copy revives its source's own tombstone

Found draining the live night's queue after P2 (cairn/changes/the-live-night-findings, F1): himalaya moved a message out of nv-copy and copied it back from INBOX, both queued before a sync pushed the move, and the copy parked for good, "binding stalwart/imap/nv-copy/nv-abs-10@example.org on source stalwart/imap holds handle 4 and this write carries \1nv-abs-10@example.org: a binding pins one handle". Nothing was lost (parked, server untouched).

Reproduced on a fresh copy of round 1's re-run store (the night's store with row 53 applied) with `pimdir` at b0000aa: `pimdir queue retry -y 58`, `retry -y 60`, then a drain (`pimdir --source stalwart/imap tag --add '$f1probe' 'mid:nv-abs-10@example.org and collection:"stalwart/imap/INBOX"'`): both parked again, attempts 3, same error. nv-copy held the identity as a tombstone (`deleted` 1) bound to handle 4 with a base.

Cause: `PimdirMutate` counted no tombstone as holding a key, so the copy claimed the bare key and staged a create under its provisional handle; the write then met the source's own binding of that key under handle 4 and refused the rebind. pimdir STORAGE §9 has the copy revive such a tombstone; SYNC.md §7 now says how (under the handle it is bound to). An `Add` met the same refusal over its own tombstone.

## What landed

- src/mutate.rs: the target read sorts what it finds: a tombstone agreeing with the carried message (`summary::same_message`) goes to `revivable`, any other row to `held`. A create claiming a revivable key revives the tombstone (`mutate::revived`: its handle and base kept, `Dirty`, the body, summary, sort key and flags carried, no origin) and stages no create; a `Move` doing so leaves its own tombstone with no destination. An `Add` revives its own agreeing tombstone, and over one of another message claims a minted key through the same reading loop, the coroutine's collection as the target (`create_writes` takes the key).
- src/client/producer.rs: the drain stages a queued `add` without the coroutine, so `over_own_tombstone` does the same there: the source's row under each key read in turn, an agreeing tombstone revived, any other row minted past. tests/duplicate_link_id.rs `a_queued_add_over_its_sources_tombstone_revives_it` and `a_queued_add_of_another_message_beside_its_sources_tombstone_is_minted` (a bound message removed through the queue, then the same message, and another one reusing its `Message-ID`, added through it) parked before on "Pimdir binding INBOX/saved@x on source remote holds handle 4 …: a binding pins one handle".
- tests/queue.rs `a_store_failure_parks_the_row_and_the_pass_continues` provoked its store failure with that very add (an add of an identity the source holds as a tombstone, parked as a refused rebind), which pinned what STORAGE §9 calls a revive; it now provokes it with a copy into a target collection with no declared kind, another of STORAGE §15.2's examples, its assertions otherwise unchanged.
- vectors/sync/216 to 219 vendored at the pimdir revision spec/REVISION names; they failed on a snapshot of b0000aa ("the run failing differs", left false, right true; the drain "items rows differ" for 216 and 217).
- tests/engine/membership.rs `a_copy_over_a_member_moved_out_unpushed_revives_it`: copy, land, move out to a trash unpushed, copy again; failed before with the live error ("binding archive/msg-a on source left holds handle i1-copy and this write carries \u{1}msg-a: a binding pins one handle"). src/mutate.rs `a_tombstone_holding_another_message_mints_a_copy` (failed before: the copy took the bare key) and `a_tombstoned_holder_does_not_block_a_copy` now asserting the revive.
- Re-run on a fresh copy of the night's store (.lanes/live/stores/neverest-step1), every parked row retried and drained with the fixed `pimdir`: 51, 53, 58, 59 and 60 apply (58 and 60 revive nv-copy's row under handle 4, 53 and 59 move it on as Deleted Items' fourth and fifth keys); 55 and 57 park "unknown seq: 19", moves of a row already moved out when they apply, which is right. `pimdir check` finds the store consistent.

## Spec

[mutate](../spec/mutate.md): *A staged create never takes a key its target holds* revives the source's own agreeing tombstone and mints beside one of another message, for an `Add` too; new scenarios *A copy over a member moved out, the move not pushed* and *A tombstone holding another message*. *Add stages a locally-authored create* points there.
