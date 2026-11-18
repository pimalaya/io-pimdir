---
cairn: log
change: one-recognition-of-a-minted-key
landed: 2026-11-18
---

# One recognition of a minted key

CLI family analysis of 2026-11-15, PIM-13, re-checked at d7cfe5e: the engine looked a pending create up by the hint and the key minted over its provisional handle, `is_landable` cut the hint out of a minted key by its length, and `starts_with("dup:")` stood in sync.rs and upgrade.rs, beside the rekey's `minted_from`.

## What landed

- src/placement.rs: `PimdirLinkId::landed_hint` (read from `minted_from`) and `is_minted`; `is_landable` reads `landed_hint`.
- src/sync.rs: `creates_of(hint)` serves `disagreeing_create` and `take_pending_create`, the create keyed by the hint first; sync.rs and upgrade.rs ask `is_minted`.
- Test: src/placement.rs `a_key_names_the_hint_that_lands_it_or_none`. No behaviour change: every vector and the whole suite pass as before.

## Capabilities moved

sync ("A minted key is recognised once", added).

In the working tree on io-pimdir d7cfe5e, uncommitted, no version bumped; CHANGELOG.md [Unreleased] names it.
