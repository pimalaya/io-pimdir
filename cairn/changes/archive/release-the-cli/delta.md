---
cairn: delta
change: release-the-cli
---

# Delta

## ADDED Requirements

### Requirement: A tag ships the binary and publishes the crate
A tag SHALL produce a `pimdir` binary per released platform, carrying its manual pages, completion scripts and JSON Schemas, and SHALL publish the crate. The shared pimalaya/nix workflows do both, so the flake SHALL expose the `cross-<target>` outputs they build and the derivation SHALL leave `result/bin/pimdir` and `result/share` where they collect them. The workflow's project name is the binary's, `pimdir`, not the crate's.

A released binary SHALL be built with `cli` and `vendored`: the binary is behind a feature that is deliberately not a default, and it carries its own SQLite so it depends on no library where it lands. A build asked for `cli` alone SHALL still run, which under nix means an explicit rpath onto the system SQLite, rustc's `-L` alone leaving none.

## MODIFIED Requirements

None.

## REMOVED Requirements

None.
