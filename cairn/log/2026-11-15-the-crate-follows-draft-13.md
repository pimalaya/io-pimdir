---
cairn: log
change: the-crate-follows-draft-13
landed: 2026-11-15
---

# The crate follows draft-13

spec/ is pimdir at 627271b: storage-draft-13, sync-draft-13 and search-draft-13, which carry a-refused-push-shows-on-its-receipt (cairn/log/2026-11-15-a-refused-push-shows-on-its-receipt.md), decided by the owner on 2026-11-15. Draft-12, tagged earlier the same day, was never released by this crate, so its changelog entry names draft-13 instead.

## What moved

- `sql::FORMAT_VERSION` is `storage-draft-13`, `sync::SYNC_VERSION` `sync-draft-13`, `sql::search::FORMAT_VERSION` `search-draft-13` and `sql::search::FORMAT_DRAFT` 13; README.md states the conformance at draft-13.
- The version tests: draft-12 joins the earlier drafts, and the later draft they try is draft-14 (src/version.rs, tests/store_version.rs, tests/open_current.rs, tests/cli_owner.rs, tests/search.rs).
- Capabilities moved: store, search, cli and spec-fidelity, as for draft-12.

Breaking: a store or a sync state of an earlier draft is refused, never migrated, and an index of an earlier draft is rebuilt.

Checked: `cargo fmt --check`, `cargo clippy --all-features --all-targets -D warnings`, the suite in CI's three configurations, tests/spec_drift.rs against pimdir 627271b.
