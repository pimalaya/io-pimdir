---
cairn: log
change: file-sources
date: 2026-10-10
---

# File sources pass their vectors

Implements pimdir's file-sources (draft-04, untagged): sync vectors 55 to 61, a Nextcloud-shaped folder named at `Meta`, fetched, edited on either side, conflicted, renamed and deleted.

- The engine needed one file-specific line: `summary::without_body` keeps a held file's size over a listing, as it does a message's, so a pushed local edit keeps its body's summary (vector 58 failed before).
- tests/vectors_sync.rs builds a file member's meta, and a file fetch's, from the `link_id` and `summary` the case states (`stated_file`).
- Capabilities moved: sync (files as a mutable kind), summaries (A.7 size).

## Verification

`cargo test --all-features` with every sync vector, the seven new ones included, `--no-default-features --features cli`, clippy `-D warnings`, `cargo doc` and `cargo fmt --check` green.
