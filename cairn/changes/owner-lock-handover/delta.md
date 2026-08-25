---
cairn: delta
change: owner-lock-handover
---

## ADDED Requirements

### Requirement: Releasing the owner role and retaking it is one operation
The process-wide owner lock SHALL be released and reacquired under one critical section: the registry owns the locked file description and closes it as the last handle goes, rather than tracking a weak reference while each handle holds its own.

A strong count reaches zero before the description it named is closed, so a registry that let the two be observed apart would have the next handle open a second description and `flock` itself out of its own store, reporting `Owned` with no other process in it (pimdir SPEC §8).

#### Scenario: Handing the role over inside one process
- GIVEN a store whose owner handles are taken and dropped concurrently
- WHEN a handle is taken as the last one is being released
- THEN it shares the role, and no acquisition reports the store owned

## MODIFIED Requirements

None.

## REMOVED Requirements

None.
