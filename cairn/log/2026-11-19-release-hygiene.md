---
cairn: log
change: release-hygiene
landed: 2026-11-19
---

# Release hygiene

The release audit of 2026-10-30 is answered. The docs, the CI guard on `search`, `cargo deny --all-features`, the package `exclude`, the libz-sys update and the pimdir checkout at the tag the crate claims (now the revision spec/REVISION names) landed with the earlier releases. The `[patch]` of ical-rs and pimalaya-cli was dropped in 3494c25 (2026-11-15): Cargo.lock takes ical-rs 0.7.1 and pimalaya-cli 0.6.0 from crates.io and holds no git source. 0.16.0 and 0.17.0 were released since, so the [Unreleased] read is the one each release made; it was empty at e78ef92.

Capabilities moved: none (docs, CI, lockfile and packaging only).
