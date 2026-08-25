---
cairn: delta
change: rekey-carries-the-spine
---

## ADDED Requirements

### Requirement: A superseded handle licenses its own rebind
A write batch dropping a handle with `ReplicaDropReason::Superseded` SHALL let the binding holding it move to whatever handle the same batch upserts for that identity, by deleting the binding and inserting the new one (pimdir SPEC §10, §12). A batch dropping a handle for any other reason, or not dropping it at all, SHALL keep the bound handle and record the incoming one as ambiguous.

The licence SHALL be per handle rather than per batch: a rebuild carrying a genuine second copy of an identity still freezes that one.

#### Scenario: A renumbered collection is not a duplicated one
- GIVEN a binding on a handle a rekey batch supersedes
- WHEN the batch drops it and upserts the item under a new handle
- THEN the binding follows the new handle, projects `Clean`, and records no ambiguity

## MODIFIED Requirements

None.

## REMOVED Requirements

None.
