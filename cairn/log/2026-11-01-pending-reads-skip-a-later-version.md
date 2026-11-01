---
cairn: log
date: 2026-11-01
change: pending-reads-skip-a-later-version
---

# Pending reads skip a later payload version

Found by the cardamum port to bff5737 (cardamum cairn/changes/pimdir-follows-storage-draft-08): `pending_actions`, and through it `pending_creates` and `count_pending_creates`, decoded every pending row strictly and failed on one stating a later payload version (`UnknownVersion(Some(_))`). The drain skips such a row and leaves it pending for a later owner (STORAGE §15.2), so one row a later producer queued failed every frontend's status read for good. Every pending read now skips it as the drain does; a malformed row still fails the strict read until the drain parks it, and the overlay still skips both.

The `store-version` and `sync-version` messages said to move the store aside and sync a new one, skipping the step that keeps the writes the store alone holds: they now say to let the sync engine that wrote it push what it queued first. The himalaya, cardamum, calendula and neverest ports had each added that step on top.

Trivial fix, landed without a proposal. Moved: [store](../spec/store.md) (the overlay paragraph). Test: tests/store_rules.rs `a_reader_skips_a_row_of_a_later_payload_version`, failing first with `Action(UnknownVersion(Some(2)))`.
