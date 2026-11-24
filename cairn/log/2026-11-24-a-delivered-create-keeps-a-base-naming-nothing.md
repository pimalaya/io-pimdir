---
cairn: log
change: a-delivered-create-keeps-a-base-naming-nothing
landed: 2026-11-24
---

# A delivered create keeps a base naming nothing

Asked by the lead (cairn/changes/the-live-night-findings, F4): pin the revive f7d10db gives a tombstoned delivered create. The pin showed a bug, older than the revive: `PimdirHub::absorb_upsert` gives a binding of an immutable kind the item's body as its base whenever the base states no revision, a delivered create's included. SYNC §2 has a delivered create's base name no flag set, revision nor body, and `absorb_landing` lands only such a create on the handle a relocation states (SYNC §4). A flag change staged on a delivered create, or a copy reviving one tombstoned, so left it bound under its provisional handle with a base naming the body, and the relocation passed it by.

## What landed

- src/hub.rs: the adoption skips a delivered placement (`!placement.delivered()`).
- pimdir vectors 220 (a copy reviving a tombstoned delivered create) and 221 (a flag change on a delivered create), vendored; both failed on f7d10db, by the engine and by the drain ("bindings rows differ": `base_object` m1 where null). On b0000aa vector 220 failed otherwise: `base_present` 0, the copy restaging a pending create under the same handle, a second `Add` to push.
- tests/relocation_landing.rs `a_delivered_create_flagged_since_lands_all_the_same`: a delivered create, a queued flag change drained, then a relocation stating handle 7. Without the guard the base named the body (`left: Some(PimdirHash(…)) right: None`), and with that assertion set aside the landing was lost (the handle stayed `\u{1}a@x`, not `7`).

## Spec

[hub](../spec/hub.md): *An immutable binding owes no body* leaves a delivered create's base naming nothing; new scenario *A delivered create flagged, then landed*.
