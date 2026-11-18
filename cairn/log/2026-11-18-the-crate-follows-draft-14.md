---
cairn: log
change: the-crate-follows-draft-14
landed: 2026-11-18
---

# The crate follows draft-14

spec/ is pimdir at 587d48e: storage-draft-14, sync-draft-14 and search-draft-14, carrying pimdir's cairn/changes/the-draft-14-lines (D34, PIM-12, PIM-14, D6, C1).

## What moved

- `sql::FORMAT_VERSION` is `storage-draft-14`, `sync::SYNC_VERSION` `sync-draft-14`, `sql::search::FORMAT_VERSION` `search-draft-14` and `sql::search::FORMAT_DRAFT` 14; README.md states the conformance at draft-14.
- The version tests: draft-13 joins the earlier drafts a store or an index is refused or rebuilt at, and the later draft they try is draft-15 (src/version.rs, tests/store_version.rs, tests/open_current.rs, tests/cli_owner.rs, tests/search.rs).
- src/sync.rs: a create the store offered, refused for good, is upserted as the source's pending create before the `RefusePlacement` (SYNC §5, D6): vector 171, failing on 58f907e ("bindings rows differ"), passes. Vectors 168 to 170 passed on 58f907e already: the hub projects a deleted row `Tombstone` (PIM-12) and `move-in-place` landed on 2026-11-17 (PIM-14).
- src/remote.rs: `PimdirPushOutcome` documents the refusal exceptions of D34 (401, 407, 423 and a quota exceeded are no refusal); the classification is each connector's.
- tests/versions.rs runs vectors/versions.json through `PimdirFormatRefusal::order`.
- Capabilities moved: store, search, sync, cli and spec-fidelity.

Breaking: a store or a sync state of an earlier draft is refused, never migrated, and an index of an earlier draft is rebuilt.
