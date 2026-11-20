---
cairn: log
change: the-crate-follows-draft-15
landed: 2026-11-20
---

# A hash key names its bytes

cardamum (src/pimdir/backend.rs `create_card`) and calendula (src/pimdir/backend.rs:291) derive an `add`'s link id from the body they hand to `enqueue_with_body`, which then takes the `X-PIMDIR-` properties out and stores what remains (STORAGE §18). A UID-less card or event was so queued under a `hash:` key over bytes the store never held, which the first read of its member retires (STORAGE §9, §10). pimdir draft-15 states the rule (STORAGE §15.3, SYNC §7, vectors/sync/176 and 177; pimdir cairn/log/2026-11-20-a-hash-key-names-its-bytes.md).

- src/client/producer.rs: `fresh` refuses an `add` stating a `hash:` key the body in hand does not give, as `PimdirError::HashKeyMismatch` (`hash-key-mismatch`), before anything is appended; `enqueue_with_body` checks it against the stripped body. `PimdirEnqueued::link_id` answers the key an `add` was checked under, so a producer states none and reports the one the store will hold. The drain parks an `add` whose stated `hash:` key its stored body does not give.
- src/mutate.rs: an `Add` stating a `hash:` key its `body` does not give fails as `PimdirMutateError::HashKeyMismatch`, staging nothing.
- tests/capabilities.rs `a_uidless_resource_is_keyed_over_the_bytes_it_is_queued_with` (the consumers' pattern refused; with no key stated, `link_id` answers the key of the stripped bytes) and `the_owner_parks_an_add_stating_another_bodys_hash_key` fail with the checks disabled and pass; vectors/sync/176 fails with the engine's check disabled, 177 passes either way. tests/vectors_sync.rs reads format 10.
- spec/vectors re-vendored from the pimdir working tree; spec/generator and spec/REVISION at pimdir 39a0f77.

Left: cardamum and calendula pass `link_id: None` and report `PimdirEnqueued::link_id`, each in its own lane.

Capabilities moved: store, mutate, sync.
