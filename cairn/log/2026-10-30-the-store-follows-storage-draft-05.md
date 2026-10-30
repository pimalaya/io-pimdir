---
cairn: log
change: the-store-follows-storage-draft-05
date: 2026-10-30
---

# The store follows storage-draft-05

The store side of pimdir's 2026-10-30 edits (a-merge-keeps-every-body, should-fix-before-the-freeze, a-read-body-is-verified, every-vocabulary-is-open-or-closed, the SEARCH `has:attachment` fix), the orphan blob two apps hit, and the audit's S1. Each rule has a test that failed first; the engine side stays with the next wave.

## What landed

- src/client/write.rs, src/sql.rs: `bindings.shared_object` pinned in the refcount diff and in `REFCOUNT_DRIFT` (tests/shared_object_pin.rs rewritten to the rule; without the pin it fails with drift).
- src/client.rs: `delete_collection` maps `collections_keep_queued` to `PimdirError::Queued` (`collection-queued`); `set_collection_role` refuses another kind's role as `PimdirError::Role` (`collection-role`); src/client/reader.rs reads such a role as none (`collection::role_of_kind`).
- src/placement.rs, src/summary/*: `PimdirLinkId::RESERVED`, `PimdirLinkId::from_hint`, used for `Message-ID` and `UID`.
- src/codec.rs, src/client/producer.rs: `PimdirAction::Update { if_match }`, parked on mismatch naming both bodies; an unknown payload `v` skipped by the drain.
- src/client/blobs.rs: `get` checks a body against its name; `discard_unverified`, run by the drain's owner under the exclusive staging lock; a per-writer temporary name, removed on failure.
- src/client/producer.rs, src/client/capability.rs: `enqueue_with_body`, the gate taking the bytes in hand.
- src/client/search.rs, src/client/search/compile.rs, src/client/search/extract.rs: `FMTTYPE` in a calendar item's `attachment` field, `has:attachment` reading the field's token count from `object_text_docsize` (`pimdir_column_size`).
- Tests: tests/store_rules.rs (five, each failing first: a raw `Sql` error, parked, applied, the flipped bytes served, the other writer's temporary renamed away), tests/staging.rs and tests/capabilities.rs for `enqueue_with_body`, the summaries and search vectors (item 23 missing before); fake hashes replaced by real ones in tests/queue.rs, conflict.rs, retention.rs, references.rs, capabilities.rs and engine/common.rs, a body now being read through its name.
- Capabilities moved: store ("Blobs are content-addressed and sharded", "A blob rename is durable", "A collection can be deleted", "A collection keeps the role its source states", "A binding persists what it last agreed with the hub on", "The store repairs what it can recompute", "The queue derives summaries from bodies", "An owner skips the actions it cannot apply", modified; "A stated identity beginning like a derived key is escaped", "An edit names the body it replaced", "A write staged with its body stages nothing when refused", added); search ("A calendar item has an attachment when its field is not empty", added).

## Migration for the frontends

- `PimdirAction::Update` gains `if_match`: cardamum src/pimdir/backend.rs:255, unfog src/pimdir/backend.rs:439, calendula src/pimdir/backend.rs:338, neverest src/conflict.rs:188, src/offline/driver.rs:4381, tests/multikind.rs:283, tests/common/mod.rs:325 state the body they read (`Some(hash)`), or `None`.
- A create or an edit holding its bytes goes through `producer.enqueue_with_body(collection, body, |hash| action)` in place of `stage_body` then `enqueue` or `enqueue_add`, so a refused one leaves no body.
- Matches on `PimdirError` gain `Foreign` (`store-foreign`), `Queued` (`collection-queued`) and `Role` (`collection-role`); the enum is `#[non_exhaustive]`.
- A connector naming a stated identity to its server strips one leading `esc:` (neverest, next wave).
