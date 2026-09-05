---
cairn: change
id: revised-standard-alignment
status: landed
created: 2026-09-05
---

# Alignment with the standard's algorithm audit

## Why

The standard was audited on 2026-09-04 (pimdir/cairn/changes/algorithm-audit-2026-09-04/) and revised in nine changes on 2026-09-05: the change-feed cursor and stamps, the collector and the producers' pins, the queue's order and rollback, deletes and the trash, fallback keys and provisional handles, the projection gating on the item conflict, compaction before the freeze, search roles, checks and the ledger. Re-vendored, the schema and statements no longer matched the std client, and the engine still carried what the audit removed or restated: a `KeepBoth` policy no `UID`-keyed server can land, a configured delete policy where the answer is a fact about the collection, a caller-supplied placeholder where the standard fixes the provisional handle, a create pushed ahead of the probes it may be, a vanished local edit dropped, an item-level conflict projected on one binding and never cleared, a rekey dropping after it upserts, a base a `Full` fetch left behind, and a link taken on a `Message-ID` alone.

## What

Every divergence above, engine and std client: the vendored spec byte for byte with its five new statements and four removed, the schema's triggers in the check, the store-wide drain with bounded retries, the pinned enqueue, the recomputing collector refused in flight, the trash of every deleted row, the rename retargeting the queue, `delete_collection`, `list_item_conflicts`, `lookup_objects` with sizes; in the engine the provisional handle, the vanished edit re-staged, the create waiting for probes, the refused delete decided beside the collection's sources, the item-level conflict held and settled by an `Edit` or a `Remove`, the fetch moving the base, the restated hint keyed afresh, the size witness, the rebuild's drops first with the hub keeping the dropped base, the pairing by revision, and the item-level conflict a rebuild carries with no revision. Every suite runs the revised vectors, thirty-two of them, and the property suites converge again.
