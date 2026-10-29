---
cairn: log
change: gtd-hardening
date: 2026-10-29
---

# Filing a task never loses or doubles it, readers agree on older stores

io-pimdir's part of unfog's cairn/changes/gtd-hardening (pimdir's of the same id), with its follow-up of the same day.

- **store** (spec/store.md): `PimdirAction::Add::supersedes` (`codec::PimdirSupersedes { collection, seq }`), applied by the drain with the superseded item's tombstone in one transaction and recorded (`supersede_item`), a parked add removing nothing, the gate reading the add's capabilities and the superseded remove's; `capability::X_VTODO` and `PimdirReader::accepts_vtodo`, a VTODO refused where a source states `none`; `PimdirReader::replied`, the message ids a live mail outside `sent` and `drafts` replies to; `PimdirStore::open_current`, an owner open refusing an absent or older store (`PimdirError::NotCurrent`) rather than migrating it.
- **sync** (spec/sync.md): `PimdirLoaded::held`, the tombstones whose remove waits for the add superseding them, and `PimdirLoaded::superseded` with `PimdirChangeKind::Add::supersedes` (`placement::PimdirSuperseded`), the member a create keeping the superseded identity may be moved from, so a connector able to move delivers it with a `MOVE` and a server keeping one `UID` per account never holds it twice.
- **summaries** (spec/summaries.md): a reader reads the task columns an owner has not reconciled yet as `NULL`, so a store written by an older build is listed, not refused.
- spec/ re-vendored from pimdir (0a70d99 there and after): `supersessions`, `supersede_item`, `list_superseded_pending`, `list_superseded_bindings`, `list_replied`, the new `rename_queue_targets`.

Breaking: struct literals of `PimdirAction::Add`, `PimdirChangeKind::Add` and `PimdirLoaded` name the new fields. No version bump yet, the consumers pinning the git revision until the provider tests are done.

Capabilities moved: store, sync, summaries.

Verified: `cargo test --all-features` 635 tests green, fmt and clippy clean on all features (the `PimdirArg` variant sizes allowed, with the reason); regression tests failing on the earlier code first: tests/older_task_summary.rs, tests/engine/supersedes.rs (a parked add, a refused push, a filing moved on a server keeping one identity, which the code before the follow-up leaves unmoved), tests/open_current.rs, tests/replied.rs, and pimdir's vectors/sync 71, 72 and 73.
