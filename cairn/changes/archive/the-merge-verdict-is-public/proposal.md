---
cairn: change
id: the-merge-verdict-is-public
status: landed
created: 2026-11-19
---

# The merge verdict is public

## Why

Owner ruling of 2026-11-19. The Android app's A8 fix (android rust/src/offline/merge.rs, android cairn/log/2026-11-19-the-audit-lines-a7-to-a11.md) restated the rules of the private `PimdirHub::reconcile_content` in its own `lost` and `diverges`, because what the hub decided (`overruled`, `diverged`) is `pub(crate)`: a second copy of SYNC §9's three facts and two rules, which the next rule change would have to land twice.

## What

- `PimdirHubItem::reconcile(&mut self, source, placement, policy) -> PimdirHubVerdict`: the former `reconcile_content` body, unchanged, as a public, I/O-free method of the item. The hub's `absorb` calls it, so there is one implementation.
- `PimdirHubVerdict { diverged, kept_shared, lost, unsummarized }`, `PimdirHubLoser { object, side }` and `PimdirHubLoserSide::{Shared, Incoming, Diverging}`, the former private verdict types, public with the hub prefix.
- A plain method, no trait: the policy is a value the caller passes (coroutine-over-trait). A tombstone and a person's settle stay the hub's paths around it, as `absorb_upsert` reads them; the app already guards both.
- The app can replace `lost` and `diverges` with one call on a clone of the item it builds, reading `verdict.lost` (object and side) and `verdict.diverged`. That is the app's change, not made here.
