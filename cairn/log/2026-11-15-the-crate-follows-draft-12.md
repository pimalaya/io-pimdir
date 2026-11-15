---
cairn: log
change: the-crate-follows-draft-12
landed: 2026-11-15
---

# The crate follows draft-12

spec/ is pimdir at acd00be: storage-draft-12, sync-draft-12 and search-draft-12, which carry the two rules this crate already follows (a-narrowed-round-proves-only-what-its-enumeration-lacks, a-pending-create-is-withdrawn-whatever-the-rights), the owner taking reading (a) of the first on 2026-11-15. Re-vendoring moved two statements, `init_store_meta` and `init_index_meta`, each writing the new draft's name.

## What moved

- `sql::FORMAT_VERSION` is `storage-draft-12`, `sync::SYNC_VERSION` `sync-draft-12`, `sql::search::FORMAT_VERSION` `search-draft-12` and `sql::search::FORMAT_DRAFT` 12, by which an index of an earlier draft is told from a later one; README.md states the conformance at draft-12.
- The version tests: draft-11 joins the earlier drafts a store or an index is refused or rebuilt at, and the later draft they try is draft-13 (src/version.rs, tests/store_version.rs, tests/open_current.rs, tests/cli_owner.rs, tests/search.rs).
- Capabilities moved: store (the fresh store's name, the sync version, the refused markers), search (the index version, the earlier ones rebuilt), cli (the store of another draft), spec-fidelity (the revision and the conformance line).

Breaking: a store or a sync state of draft-11 is refused, never migrated, and an index of draft-11 is rebuilt.

Checked: `cargo fmt --check`, `cargo clippy --all-features --all-targets -D warnings`, the suite in CI's three configurations, tests/spec_drift.rs against pimdir acd00be. spec/REVISION moves to the tagged commit once the tags are made.
