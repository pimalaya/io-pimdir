---
cairn: delta
change: one-recognition-of-a-minted-key
---

## ADDED Requirements

### Requirement: A minted key is recognised once
`PimdirLinkId::landed_hint` SHALL answer the hint whose arrival lands a create of the key (SYNC §3, §6): the key itself when it is no minted one (`is_minted`, `dup:` leading it), else the hint among `minted_from` that is no minted key and mints it over its own provisional handle, else none. `is_landable` SHALL read it, and the engine SHALL find the pending or delivered create a listed hint lands, to land it or to report the member minted apart from it, by it alone, the create keyed by the hint first. The rekey's copies stay `minted_from`'s, over any handle.

#### Scenario: A hint holding `#`
- src/placement.rs `a_key_names_the_hint_that_lands_it_or_none`: `a#b@x` and its key minted over its provisional handle name `a#b@x`; one minted over `7#8`, that key minted again, and a key minted twice over its own handle name none

## MODIFIED Requirements

## REMOVED Requirements
