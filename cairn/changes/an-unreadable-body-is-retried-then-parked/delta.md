---
cairn: delta
change: an-unreadable-body-is-retried-then-parked
---

## MODIFIED Requirements

### Requirement: The queue derives summaries from bodies
Appended: a body an `add` or `update` names that the blob store lacks SHALL fail the row as a failure of the environment (`PimdirError::Io`, `NotFound`), never apply it: its attempts are bumped and it stays pending, then parks past `MAX_ATTEMPTS` (pimdir STORAGE §15.2).

#### Scenario: A body unlinked after its enqueue
- GIVEN a queued `add` whose blob was unlinked after the enqueue
- WHEN the owner drains `MAX_ATTEMPTS` times
- THEN each drain but the last counts it `retried`, the last parks it with the error naming the body, and no item was created (tests/queue.rs `a_queued_add_missing_its_body_is_retried_then_parked`)
