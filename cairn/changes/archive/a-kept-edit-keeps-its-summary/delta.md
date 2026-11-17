---
cairn: delta
change: a-kept-edit-keeps-its-summary
---

## ADDED Requirements

### Requirement: A kept local edit keeps its summary
A listing naming a member at its base revision SHALL refresh the summary and sort key of its placement only when the placement holds no local edit of a mutable kind (`staged_edit`): the listing describes the server's body, and a placement whose local body the run keeps, pushed or still pending, keeps that body's summary (SYNC §5). A rekey carrying such an edit the remote did not edit keeps its summary and sort key the same way.

#### Scenario: A renamed contact
- GIVEN a synced card edited offline, its edit stating its own summary and sort key
- WHEN a full round lists the member at the unchanged revision, with push off or on, or the server renumbers and the collection is rekeyed
- THEN the placement keeps the edit's summary and sort key (tests/engine/summary_kept.rs)

## MODIFIED Requirements

## REMOVED Requirements
