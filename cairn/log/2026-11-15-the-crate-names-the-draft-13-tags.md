---
cairn: log
change: the-crate-names-the-draft-13-tags
landed: 2026-11-15
---

# The crate names the draft-13 tags

pimdir tagged storage-draft-13, sync-draft-13 and search-draft-13 on 2026-11-15, all three on its commit 1c6442e, which pins this crate at f38ccb6 in checks/io-pimdir.rev. spec/REVISION named 627271b, the commit that moved the vendored directories, which no tag names.

spec/REVISION now names 1c6442e. The four vendored directories are the same bytes at both commits, so nothing under spec/ moved, and tests/spec_drift.rs guards the copy against the tagged commit. cairn/spec/spec-fidelity.md and CHANGELOG.md name the tags.
