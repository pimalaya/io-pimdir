---
cairn: log
change: a-fourth-copy-takes-a-key-of-its-own
landed: 2026-11-24
---

# A fourth copy takes a key of its own

Found by the live test night of 2026-11-23 (cairn/changes/the-live-night-findings, P1 and P2): himalaya copied one message from INBOX into a folder and trashed it, again and again through its pimdir queue, and neverest parked the fourth copy into one collection for good, "binding stalwart/imap/Deleted Items/dup:dup:nv-abs-10@example.org#…#… on source stalwart/imap holds handle 4 and this write carries …: a binding pins one handle". Nothing was lost (parked, server untouched).

Reproduced on a copy of the scratch store .lanes/live/stores/neverest-step1 with `pimdir` at 65a5026: `pimdir queue retry 53`, then a drain (`pimdir --source stalwart/imap tag --add …`), parked row 53 again (attempts 2, same error). Deleted Items held the message under `k`, `dup:k#\1k` and `dup:dup:k#\1k#\1k` (handles 2, 3, 4).

Cause: `PimdirMutate` read the target by `PimdirLoadSelection::Links([k, dup:k#\1k])` only, so `PimdirLinkId::claim` never saw the second mint held and returned it; the write then refused to repoint its binding from handle 4 to the provisional one.

pimdir 2026-11-24 adds vector 215, vendored here at that revision; no part's text moves (sync-draft-15).

## What landed

- src/mutate.rs: `PimdirMutate` keeps the keys it asked the target for (`asked`) and those a live row holds (`held`); while the claimed key is one it did not ask for, it yields another `WantsLoad` of that key alone and claims anew.
- src/client/producer.rs: the drain services every load a staged create asks for (`while let`, was `if let`).
- tests/engine/membership.rs `a_fourth_copy_into_a_collection_takes_a_key_of_its_own`: failed before the fix with the live error ("Pimdir binding archive/dup:dup:msg-a#\u{1}msg-a#\u{1}msg-a on source left holds handle app-1 and this write carries \u{1}dup:dup:msg-a#\u{1}msg-a#\u{1}msg-a: a binding pins one handle"); vectors/sync/215 failed too, by the engine ("the run failing differs", left false, right true) and by the drain ("bindings rows differ").

## Spec

[mutate](../spec/mutate.md): *A staged create never takes a key its target holds* reads the target again for each key minted over a held one; new scenario *A fourth copy into a collection holding three*.
