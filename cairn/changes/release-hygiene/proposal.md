---
cairn: change
id: release-hygiene
status: active
created: 2026-10-30
---

# Release hygiene

## Why

The release audit of 2026-10-30 (report-io-pimdir.md, blockers 1 and 9, should-fix "API and release engineering", nice-to-have) found the crate unpublishable and some of its docs and checks out of step with the code:

- docs describing a refusal the code no longer makes: README.md:43 (the SEARCH index "is not implemented yet"), cairn/spec/store.md:287-299 (older stores refused with `Stale`, while the owner open reconciles), src/client/schema.rs:522-525 ("never reconciled here"), tests/stale_store.rs:1 ("refused, never reconciled");
- the CI guard proving the spec suites ran omits `search` (.github/workflows/tests.yml:55), so a missing spec checkout lets tests/search.rs pass silently;
- the package ships AGENTS.md, CLAUDE.md, deny.toml and the nix files (`cargo package --list`);
- `cargo deny --all-features check` fails on RUSTSEC-2026-0418 (libz-sys 1.1.29 via git2 in the pimalaya-cli build dependency), and CI runs deny on the default features only, so the `cli` and `search` dependencies are never checked;
- the crate claims pimdir draft-04, which is untagged, and CI checks pimdir out with no `ref`, so every spec push silently changes what the crate is tested against;
- Cargo.toml `[patch]`es ical-rs and pimalaya-cli to commits past their last tags, which `cargo publish` drops.

## What

- Docs say what the code does: the reference index ships behind `search`; the owner open reconciles an older draft's store, refusing only one missing a core table. The wording about the version contract waits for the-store-version-contract.
- CI: the skip guard covers `search`; deny runs with `--all-features`; pimdir checked out at the tag the crate claims, once pimdir tags exist (`draft-04`, then `storage-draft-05`).
- Cargo.toml `exclude` drops AGENTS.md, CLAUDE.md, deny.toml, flake.nix, flake.lock, default.nix, shell.nix and package.nix from the package.
- Cargo.lock: `cargo update -p libz-sys`, that package only.
- At the release: `[patch]` dropped once the user released ical-rs and pimalaya-cli; CHANGELOG [Unreleased] read again as a net diff against 0.8.0.
