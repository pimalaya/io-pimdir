---
cairn: delta
change: a-move-in-place-is-refused
---

## ADDED Requirements

### Requirement: A move into its own collection is refused
A `Move` whose target is the collection its placement is in SHALL be refused, staging nothing, as `PimdirMutateError::MoveInPlace` (`move-in-place`): its tombstone would push a `Remove` of a member whose create, held without a body, could never deliver. The drain runs the mutation for a queued `move`, so such a row parks with that error, the item left live and nothing staged beside it. A `Copy` there is a second copy, minted as before. STORAGE §15.3 does not state the rule yet (draft-14).

#### Scenario: Refused in the mutation and parked in the drain
- GIVEN a message held without its body in a synced INBOX
- WHEN a move into INBOX is mutated, or queued and drained
- THEN the mutation answers `move-in-place` and the row parks naming it, the item live and `Clean` (src/mutate.rs `a_move_into_its_own_collection_is_refused`, tests/queue.rs `a_queued_move_into_its_own_collection_parks`)

## MODIFIED Requirements

## REMOVED Requirements
