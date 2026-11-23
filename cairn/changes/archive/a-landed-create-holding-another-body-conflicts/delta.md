---
cairn: delta
change: a-landed-create-holding-another-body-conflicts
---

## ADDED Requirements

## MODIFIED Requirements

### Requirement: A create waits for the page that lands it (sync)
Appended: a mutable arrival carrying a body other than the staged one lands as the both-changed case (pimdir SYNC §6), its base holding neither body nor revision, so the policy decides; landing it on the arrival's body as the base pushed the staged body over it.

#### Scenario: Two servers holding one identity under two bodies
- GIVEN a source holding a card and a second source whose server holds the same `UID` under another body, the hub offering the first as a pending create
- WHEN the second source syncs under `Manual`
- THEN the create lands conflicted, its body the staged one and the server's recorded as the divergence, the base holding no body, and nothing is pushed

## REMOVED Requirements
