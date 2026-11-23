---
cairn: log
change: what-the-consumers-need
landed: 2026-11-23
---

# What the consumers need, drop 3: the release candidate

The third drop of cairn/changes/what-the-consumers-need, after 2026-11-23-what-the-consumers-need.md (drops 1 and 2): io-pimdir 0.20.0 as a candidate, untagged, the tag and the publish being the owner's.

## What moved

- The version: Cargo.toml, Cargo.lock, default.nix and package.nix say 0.20.0 (the owner's yes at the lane's launch). CHANGELOG.md's Unreleased became `[0.20.0] - 2026-11-23`, opened by the "start over" note (drafts are exact: a store an earlier draft wrote is refused, never migrated) and the consumers' feature lines of cairn/changes/archive/the-query-split-by-domain, now landed.
- The MSRV: drop 2's rebuild used an `if let` match guard, which rust-version 1.89 refuses; rewritten, `cargo check --all-features --all-targets` passes on Rust 1.89.0.
- himalaya N6 (search.md): a query client built with `search` and no `indexer` maps the hit columns (index.hits) as one built with both does; writing them stays under `indexer`.
- himalaya N7: `PimdirReader::accounts_of` reads `sql::ACCOUNTS_AND_KINDS`, no coverage join, the answer unchanged.
- Housekeeping: 71 change folders whose work is done or someone else's moved to cairn/changes/archive/, references outside cairn/log re-pointed.

A correction to the drops 1 and 2 entry: drop 2 also has a rekey store the body a listing carries as the diverging body of a conflict over a local edit (tests/engine/rekey_cases.rs `a_conflict_stores_its_listed_body`), which that entry's R8 line leaves out.

## Checks

The CI matrix, the MSRV check, `cargo deny check`, `cargo package` and `cargo semver-checks` against 0.19.1, the differential oracle and the performance table: counts and verdicts in cairn/changes/what-the-consumers-need/journal.md and report.md.
