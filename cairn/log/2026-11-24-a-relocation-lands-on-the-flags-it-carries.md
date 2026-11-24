---
cairn: log
change: a-relocation-lands-on-the-flags-it-carries
landed: 2026-11-24
---

# A relocation lands on the flags it carries

Found by the live test night of 2026-11-23, a silent loss: through himalaya's pimdir queue, `flag set -f seen -f important` then `message move` of one message, drained by `neverest sync` on Stalwart. `UID MOVE` kept the server's `\Flagged` and answered `COPYUID`; `absorb_landing` based the landed create on the item's staged flags, so the store read `Clean` while the server differed, and the next listing pulled `\Flagged` back over the user's set on the server and in the store, exit 0, no warning.

pimdir d5645bf states the rule in SYNC §4 with vector 214, vendored here at that revision.

## What landed

- src/change.rs: `PimdirWriteOp::LandPlacement` gains `flags`, the flags the relocated member carries.
- src/sync.rs: `Pending::Remove` records `carried`, the tombstone's base flags (`Unknown` without a base, the merge then a union), and `relocated` passes them into the landing.
- src/client/write.rs and src/hub.rs: the write forwards them and `absorb_landing` bases the create on them rather than on the item's flags.
- tests/engine/common.rs: `MemRemote::states_moved`, a relocation stating the handle it gave its member, as `COPYUID` does.
- tests/engine/membership.rs `a_flag_set_staged_before_a_stated_relocation_is_pushed`: failed before the fix (`left: Known({"\\Flagged"})`, `right: Known({"$Important", "\\Seen"})`); vectors/sync/214 failed too (bindings `base_flags` `["$Important", "\\Seen"]` where `["\\Flagged"]` is expected).

## Spec

[sync](../spec/sync.md): *A relocation names the member it made* carries the flags; new scenario *A flag set staged before a move that states its member*.
