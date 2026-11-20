---
cairn: change
id: the-crate-follows-draft-15
status: active
created: 2026-11-20
---

# The crate follows draft-15

## Why

pimdir opens storage-draft-15, sync-draft-15 and search-draft-15 for its freeze plan (pimdir cairn/changes/the-freeze-plan): the three parts are released together, SEARCH included (D3), and the `pimdir` binary is outside the crate's 1.0 contract (D4). The later lines of the plan (the change-feed vectors, D2, the query language) join the same draft, and this change follows each as it lands.

## What

- spec/ re-vendored from the pimdir working tree; spec/REVISION moves to the pimdir commit carrying it once made.
- The version constants and the version tests move to draft-15 (earlier draft-14, later draft-16).
- README.md and cairn/spec/search.md: search follows the crate's versioning, no longer experimental.
- `pimdir --help`, README.md and cairn/spec/cli.md: the binary is outside the 1.0 contract.
