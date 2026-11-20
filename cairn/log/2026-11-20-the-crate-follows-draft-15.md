---
cairn: log
change: the-crate-follows-draft-15
landed: 2026-11-20
---

# The crate follows draft-15

spec/ re-vendored from the pimdir working tree of 2026-11-20 (on 3568e06, uncommitted): storage-draft-15, sync-draft-15 and search-draft-15, opened by pimdir cairn/log/2026-11-20-the-three-parts-release-together.md. Only `init_store_meta` and `init_index_meta` moved among the vendored files, each writing the new draft's name. spec/REVISION still names e188e31: it moves to the pimdir commit carrying the bump once made.

## What moved

- `sql::FORMAT_VERSION` is `storage-draft-15`, `sync::SYNC_VERSION` `sync-draft-15`, `sql::search::FORMAT_VERSION` `search-draft-15` and `sql::search::FORMAT_DRAFT` 15; README.md states the conformance at draft-15.
- The version tests: draft-14 joins the earlier drafts a store or an index is refused or rebuilt at, and the later draft they try is draft-16 (src/version.rs, tests/store_version.rs, tests/open_current.rs, tests/cli_owner.rs, tests/search.rs).
- D3 (pimdir the-freeze-plan, 2026-11-20): README.md and cairn/spec/search.md no longer call search experimental; it follows the crate's versioning, SEARCH being released with the other parts.
- D4: `pimdir --help` (`long_about`), README.md and cairn/spec/cli.md say the binary is outside the crate's 1.0 contract.
- Capabilities moved: store, search, cli and spec-fidelity.

Breaking: a store or a sync state of draft-14 is refused, never migrated, and an index of draft-14 is rebuilt.
