---
cairn: delta
change: a-delta-loads-what-it-names
---

## ADDED Requirements

## MODIFIED Requirements

### Requirement: A load states what it needs
`PimdirYield::WantsLoad` SHALL carry a `PimdirLoadSelection`. A mutation asks for the one placement it edits, or, for an `Add`, every row holding the link id it must not collide with; an upgrade asks for the handles it raises; a rebuild and `PimdirOpen` ask for the whole collection; a delta page and a round's last page ask for the `Handles` they list and find vanished, the `Links` of the hints they carry and the placements that owe a push, the last page reading its absent members from `unstamped`; a page before the last asks for the `Handles` it lists and the `Links` of the hints it carries; and a sync first asks for `Handles` naming none, its sync state alone (checkpoint, coverage, round).

#### Scenario: A delta of one message in a large collection
- GIVEN a collection of 100,000 bound placements, one of them with a local flag change, and a delta listing one new member
- WHEN the delta page is loaded
- THEN the load names the listed handle, its hint and the changed placement, never the whole collection, and the write is the one a whole-collection load would give

## REMOVED Requirements
