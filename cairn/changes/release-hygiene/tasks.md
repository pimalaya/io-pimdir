---
cairn: tasks
change: release-hygiene
---

- [x] README.md: the SEARCH index ships behind `search`
- [x] cairn/spec/store.md "Schema version" (delta folded), src/client/schema.rs `check`, src/client.rs `PimdirStore::open`, tests/stale_store.rs: the owner open reconciles; `Stale` is a store missing a core table
- [x] .github/workflows/tests.yml: `search` in the skip guard
- [x] .github/workflows/tests.yml: cargo deny with `--all-features`
- [x] Cargo.toml: `exclude` the agent, deny and nix files (`cargo package --list` checked)
- [x] Cargo.lock: `cargo update -p libz-sys` (1.1.29 to 1.1.31); `cargo deny --all-features check` passes
- [x] .github/workflows/tests.yml: pimdir checked out at the spec tag the crate claims (`storage-draft-07`, 2026-10-31)
- [x] Cargo.toml: `[patch]` dropped once ical-rs and pimalaya-cli are released (the user's): dropped in 3494c25 (2026-11-15), ical-rs 0.7.1 and pimalaya-cli 0.6.0 from crates.io (Cargo.lock), no git source left
- [x] CHANGELOG [Unreleased] read again as a net diff before the release: released as 0.16.0 and 0.17.0, [Unreleased] empty at e78ef92 (2026-11-19)
- [x] Log, CHANGELOG (the package `exclude` entry is in; the rest is CI and lockfile, no entry)
