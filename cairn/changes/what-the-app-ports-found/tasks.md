---
cairn: tasks
change: what-the-app-ports-found
---

- [x] 1. `PimdirProducer::enqueue_with_body_checked` answering `PimdirEnqueued { id, partial }`; tests/capabilities.rs `a_checked_enqueue_answers_the_partial_supports_it_passed_with` (a new API, absent before)
- [x] 2. The reader, the producer and `open_current` refuse a `user_version` 0 database holding tables as `Foreign`; tests/store_version.rs `every_role_refuses_a_foreign_database_as_foreign` (failed before: `store-uncreated`); tests/open_current.rs `another_schema_version_is_refused_and_not_migrated` now expects `store-foreign` for version 0, as the owner open answers it
- [x] 3. An action on an unbound item applied or parked, never skipped; tests/unbound_items.rs `a_queued_action_on_an_item_no_source_binds_applies_or_parks` (failed before: `(0, 0, 2)` applied, parked, skipped)
- [x] 4. `PimdirDrainReport::overruled`; tests/queue.rs `a_body_a_policy_overrules_during_a_drain_is_reported` (failed before: no field, the retained item reported nowhere)
- [x] 5. `PimdirBlobs::reader` answers a verifying `PimdirBlobReader`; tests/store_rules.rs `a_streamed_body_failing_its_name_fails_at_its_end` (failed before: the flipped bytes read Ok); the verification before a push and the attachment walk read a failing body as none
- [x] 5. The overlay folds an `update` only when its `if_match` names the body held at that point; tests/overlay.rs `an_edit_that_will_park_is_not_folded` (failed before: the edit folded)
- [x] pimdir gap reported: STORAGE §15.3 states nothing for an action on an item no source binds but `remove`
- [x] Spec: store.md folded; log 2026-10-30-what-the-app-ports-found; CHANGELOG (breaking: `PimdirBlobs::reader`'s type, `PimdirDrainReport` not `Copy`)
