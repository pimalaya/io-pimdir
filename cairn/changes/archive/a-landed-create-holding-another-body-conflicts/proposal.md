---
cairn: change
id: a-landed-create-holding-another-body-conflicts
status: landed
created: 2026-10-11
---

# A landed create holding another body conflicts

## Why

Found through neverest's 2026-10-11 audit baseline (tests/endpoints.rs on Radicale): two CardDAV endpoints holding one `UID` under two bodies before the first sync end with the source's body on both, the target's gone. Since the page names its members (`35a1c3f`), the hub's projection of the source's item is a pending create on the target, and the target's listing carrying the same `UID` lands it (SYNC §6). `land` took the arrival's body as the base and kept the staged body, so the staged body read as a local edit and was pushed over a body the two never agreed on. SYNC §6 says otherwise: for a mutable kind, "a fetched body differing from the staged one is the content axis's both-changed case".

## What

- `PimdirSync::land`: a mutable arrival (a revision) carrying a body other than the staged one leaves the base without a body or a revision, keeps the staged body and the arrival's flags, so the content axis reads both sides changed and follows the policy: `Manual` parks it with the arrival's body as the divergence, `PreferLocal` pushes gated on the arrival's revision, `PreferRemote` pulls.
- The mutable property model also resolves what parks in its archive: a move relocating a server edit lands a create holding the old body, which it used to push over the edit.

## Not in scope

`PimdirUpgrade::land`, the fetch path, keeps the old shape: it is reached by a mutable create only when a listing carries no body, and holds no policy to follow.
