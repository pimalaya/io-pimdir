---
cairn: log
change: the-crate-names-the-draft-10-tags
landed: 2026-11-13
---

# The crate names the draft-10 tags

pimdir tagged storage-draft-10, sync-draft-10 and search-draft-10 on 2026-11-13, all three on its commit ae8b231. spec/REVISION named c834ee1, the last commit that moved the vendored directories, which no tag names.

spec/REVISION now names ae8b231. The four vendored directories are the same bytes at both commits, so nothing under spec/ moved, and tests/spec_drift.rs guards the copy against the tagged commit. cairn/spec/spec-fidelity.md and CHANGELOG.md name the tags.
