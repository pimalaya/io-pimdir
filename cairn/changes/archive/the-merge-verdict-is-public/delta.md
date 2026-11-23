---
cairn: delta
change: the-merge-verdict-is-public
---

## ADDED Requirements

### Requirement: The merge verdict is public
`PimdirHubItem::reconcile(source, placement, policy)` SHALL be the one implementation of SYNC §9's three facts and two rules, I/O-free, answering a `PimdirHubVerdict`; `absorb` SHALL call it for every live upsert but a person's settle.

## MODIFIED Requirements

## REMOVED Requirements
