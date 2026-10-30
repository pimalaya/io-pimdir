---
cairn: log
change: an-unreadable-body-is-retried-then-parked
date: 2026-10-30
---

# An unreadable body is retried, then parked

The drain applied a queued `add` or `update` whose body the blob store lacked: the add created an item at `Full` naming a missing body, with no summary, so it vanished from every summary view (unfog's probe, after a collector unlinked a body written outside the staging window). pimdir STORAGE §15.2 calls an unreadable body a failure of the environment, retried then parked.

## What landed

- src/client/producer.rs: `queued_body` reads the body an `add` or `update` names for its derivation, and fails a missing one as `PimdirError::Io` (`NotFound`, naming the hash); the drain's existing retry bumps the attempts and parks the row past `MAX_ATTEMPTS`.
- Tests: tests/queue.rs `a_queued_add_missing_its_body_is_retried_then_parked` and `a_queued_update_missing_its_body_is_retried_then_parked`, both failing on 4c3353e (the drain reported the row applied).
- Capabilities moved: store ("The queue derives summaries from bodies", appended).
