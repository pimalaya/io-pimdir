---
cairn: log
change: the-default-enqueue-strips
landed: 2026-11-18
---

# The default enqueue strips instructions

pimdir audit-2026-11-12, line C2: STORAGE §18 asks a producer to strip `X-PIMDIR-` properties from a resource the user did not write as it stands, and the crate's stripping enqueue was the opt-in one, which no consumer called.

## What landed

- src/client/producer.rs: `enqueue_with_body` strips a calendar or address-book body (`summary::strip_instructions`) before hashing it; `enqueue_user_body` queues the body whole, for a resource the user wrote or whose instruction they asked for; `enqueue_foreign_body` removed. `stage_body` and `enqueue` take the body as given.
- Test: tests/capabilities.rs `a_resource_is_queued_without_its_instructions_unless_the_user_asks`, which fails on d6ca71a (the default enqueue refused the online meeting as unsupported instead of stripping it).
- Left: calendula moves the path writing an online meeting the user ticked to `enqueue_user_body`.

## Capabilities moved

store ("A resource the user did not write is queued without its instructions", modified).

In the working tree on io-pimdir d6ca71a (0.16.0), uncommitted, no version bumped; CHANGELOG.md [Unreleased] names it.
