---
cairn: tasks
change: the-store-follows-storage-draft-05
---

- [x] `bindings.shared_object` counted in the write path's refcounts, in `REFCOUNT_DRIFT` and the property test's six columns (tests/shared_object_pin.rs, which fails with drift without it)
- [x] `delete_collection` refused while queue rows exist, typed `PimdirError::Queued` (failed first as a raw `Sql` error from `collections_keep_queued`)
- [x] `esc:` escape of a hint beginning with a reserved prefix: `PimdirLinkId::from_hint`, the mail, contact and calendar derivations (the summaries vector failed first: `hash:0123456789abcdef` for `esc:hash:0123456789abcdef`)
- [x] An unknown payload `v` skipped by the drain, a missing one still parked (failed first: parked)
- [x] `if_match` on the queued `update`, encoded, decoded, the owner parking a mismatch naming both bodies (failed first: applied over the replaced body)
- [x] A verified body read: `PimdirBlobs::get` answers none for a file failing its name; the drain's owner unlinks it under the exclusive staging lock (failed first: the flipped bytes were read)
- [x] S1: a blob temporary named per writer, removed on a failed write (failed first: another writer's temporary was renamed away)
- [x] The role vocabulary open: `set_collection_role` refuses another kind's role as `PimdirError::Role`, a reader reads it as none
- [x] `PimdirProducer::enqueue_with_body`: the gate and the duplicate check on the bytes in hand, the body written in the enqueue transaction, so a refused create stages nothing
- [x] `has:attachment` over a calendar item's `attachment` field, `FMTTYPE` extracted beside `FILENAME` (the search vector failed first: item 23 missing)
- [x] Fake hashes in tests replaced by real ones where a body is now read through its name
- [x] Spec: store.md, search.md; fold, log, CHANGELOG
