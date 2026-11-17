---
cairn: log
change: the-vendored-requirement-says-on
landed: 2026-11-17
---

# The vendored requirement says on

CLI family audit of 2026-11-15, PIM-12, partly fixed at d184622: store.md said `vendored` is off by default while Cargo.toml has had it on since 0.5.1.

## What landed

- cairn/spec/store.md: the requirement states `vendored` on by default and the system link behind `--no-default-features`, which CI proves on its own.
- Left for the owner: SYNC §3 against §9 for a deleted and conflicted row (format text, draft-14).

## Capabilities moved

store ("SQLite is bundled unless `vendored` is turned off", modified from "SQLite is the system's unless `vendored` says otherwise").

In the working tree on io-pimdir dcf49e2 (0.15.1), uncommitted, no version bumped; CHANGELOG.md [Unreleased] names it.
