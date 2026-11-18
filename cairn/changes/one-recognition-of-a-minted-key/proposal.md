---
cairn: change
id: one-recognition-of-a-minted-key
status: landed
created: 2026-11-18
---

# One recognition of a minted key

## Why

CLI family analysis of 2026-11-15, PIM-13: "which held row does this hint name" was answered three times with three recognitions of a minted key: the engine's two lookups of a pending create (src/sync.rs, the hint and the key minted over its provisional handle), `PimdirLinkId::is_landable` (the key cut out by its length), and the rekey's `minted_from`; PIM-6 and PIM-7 were each a disagreement between two of them. Re-checked at d7cfe5e: the rekey already reads `minted_from` (2026-11-17), the other two still recognised a minted key apart, and `starts_with("dup:")` stood in two more places.

## What

- src/placement.rs: `PimdirLinkId::landed_hint`, the hint whose arrival lands a create of the key, read from `minted_from`, and `is_minted`; `is_landable` reads `landed_hint`.
- src/sync.rs: one `creates_of(hint)` lists the pending and delivered creates a hint lands, the one keyed by the hint first, for both `disagreeing_create` and `take_pending_create`; sync.rs and upgrade.rs ask `is_minted`.
- The rekey keeps `minted_from` for its own question, the copies minted over any handle; `landed_hint` is that list narrowed to the provisional handle. The store's join is the canonical `origin_for_link` and `destination_for_link`, which the crate runs verbatim.
- No behaviour change: every vector and test passes as before.
