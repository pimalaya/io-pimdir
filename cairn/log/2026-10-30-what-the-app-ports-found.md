---
cairn: log
change: what-the-app-ports-found
date: 2026-10-30
---

# What the app ports found

Five places the phase A ports hit (himalaya, cardamum, calendula, unfog, neverest).

## What landed

- src/client/producer.rs: `PimdirProducer::enqueue_with_body_checked(collection, body, action) -> Result<PimdirEnqueued, PimdirError>`, `PimdirEnqueued { id: i64, partial: Vec<PimdirPartial> }`; `enqueue_with_body` unchanged.
- src/client/schema.rs: `check_version` (reader, producer) and `current` (`open_current`) refuse a `user_version` 0 database declaring tables as `PimdirError::Foreign`.
- src/client/producer.rs: the drain applies or parks an action on an item no source binds, never skips it: a `remove` retains it; in a collection no source syncs, `set-flags` sets its flags (src/client/write.rs `set_unbound_flags`) and `update` writes the body as `put_item` does (a body deriving no summary or another key parks); `move`, `copy` and anything on an unbound item of a synced collection park naming it.
- src/client/producer.rs: `PimdirDrainReport::overruled: Vec<PimdirDrainOverruled>`, `PimdirDrainOverruled { id: i64, collection: String, link_id: PimdirLinkId }`, one per body a conflict policy kept as a retained item while a row was applied; `PimdirDrainReport` is no longer `Copy`.
- src/client/blobs.rs: `PimdirBlobs::reader(&hash) -> io::Result<Option<PimdirBlobReader>>`, a `Read` hashing what it streams and failing the read reaching the end with `ErrorKind::InvalidData` when the bytes do not hash to the name, with `metadata()`, `len()`, `is_empty()`; the store's own streamed reads (`PimdirSourceStore::verify`, the attachment walk) read such a body as none.
- src/client/reader.rs: the overlay folds an `update` only when its `if_match` names the body the item holds at that point of the fold.
- Tests: tests/capabilities.rs `a_checked_enqueue_answers_the_partial_supports_it_passed_with`; tests/store_version.rs `every_role_refuses_a_foreign_database_as_foreign` (failed before: `store-uncreated`); tests/unbound_items.rs `a_queued_action_on_an_item_no_source_binds_applies_or_parks` (failed before: both rows skipped); tests/queue.rs `a_body_a_policy_overrules_during_a_drain_is_reported` (failed before: reported nowhere); tests/store_rules.rs `a_streamed_body_failing_its_name_fails_at_its_end` (failed before: read Ok); tests/overlay.rs `an_edit_that_will_park_is_not_folded` (failed before: folded). tests/open_current.rs expects `store-foreign` for tables at version 0.
- Capabilities moved: store ("Blobs are content-addressed and sharded", "A store this crate may not write is never written", "A reader may overlay a collection's pending actions", "A write staged with its body stages nothing when refused", modified; "An action on an item no source binds is the store's to settle", "The drain reports the bodies a policy kept aside", added).

## Gap reported to pimdir

STORAGE §15.3 says what a `remove` of an item bound by nobody does and nothing for the other kinds; §15.2's three outcomes leave a skip the only reading, and a row skipped for ever.

## Migration for the apps

- cardamum `stage`, calendula `enqueue_with_body`, unfog `stage`: one `producer.enqueue_with_body_checked(..)?` in place of `check` plus `enqueue_with_body`, warning on `.partial`.
- neverest src/offline/remote.rs (`stored_item`, `body`, the two pushes): `PimdirBlobReader` in place of `std::fs::File` (`metadata()` and `Read` unchanged), and an `InvalidData` read reported as a corrupt body rather than pushed; the drain report's `overruled` reported beside the sync's `Overruled` events.
