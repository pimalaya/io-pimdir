---
cairn: log
change: the-crate-names-the-draft-12-tags
landed: 2026-11-15
---

# The crate names the draft-12 tags

pimdir tagged storage-draft-12, sync-draft-12 and search-draft-12 on 2026-11-15, all three on its commit e4a05b1, which pins this crate at 3fc43d7 in checks/io-pimdir.rev. spec/REVISION named acd00be, the commit that moved the vendored directories, which no tag names.

spec/REVISION now names e4a05b1. The four vendored directories are the same bytes at both commits, so nothing under spec/ moved, and tests/spec_drift.rs guards the copy against the tagged commit. cairn/spec/spec-fidelity.md and CHANGELOG.md name the tags.
