---
cairn: log
change: the-crate-names-the-draft-11-tags
landed: 2026-11-13
---

# The crate names the draft-11 tags

pimdir tagged storage-draft-11, sync-draft-11 and search-draft-11 on 2026-11-13, all three on its commit 83102f0, which pins this crate at 2b95e06 in checks/io-pimdir.rev. spec/REVISION named e443a65, the last commit that moved the vendored directories, which no tag names.

spec/REVISION now names 83102f0. The four vendored directories are the same bytes at both commits, so nothing under spec/ moved, and tests/spec_drift.rs guards the copy against the tagged commit. cairn/spec/spec-fidelity.md and CHANGELOG.md name the tags.
