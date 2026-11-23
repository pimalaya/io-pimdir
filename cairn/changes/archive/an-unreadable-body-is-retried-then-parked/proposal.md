---
cairn: change
id: an-unreadable-body-is-retried-then-parked
status: landed
created: 2026-10-30
---

# An unreadable body is retried, then parked

## Why

The drain reads the body a queued `add` or `update` names to derive its summary (`stage_action`, src/client/producer.rs), but `PimdirBlobs::get` answers `None` for a missing blob and the drain goes on: the `add` lands an item at `Full` naming a hash the blob store lacks, with no summary, so it vanishes from every view built on the summaries and `get_item` fails on it. unfog's probe reproduced it: a collector unlinked the body a frontend wrote before opening its producer (the staging-lock bug), the enqueue succeeded and the drain applied the add (report-unfog.md, blocker 1; review-app-unfog.md, item 1).

pimdir STORAGE §15.2 says a body unreadable is a failure of the environment: retried, then parked, never applied. pimdir spec edit 4 makes it a MUST with a vector.

## What

- The drain treats a body an `add` or an `update` names and the blob store lacks as an I/O failure (`PimdirError::Io`, `NotFound`): the row's attempts are bumped and it stays pending, the report counting it `retried`, and it is parked past `MAX_ATTEMPTS` with the error naming the body. Nothing of it is applied; the rows behind it proceed.
- An `add` carrying no body (`object` absent) is unchanged.
