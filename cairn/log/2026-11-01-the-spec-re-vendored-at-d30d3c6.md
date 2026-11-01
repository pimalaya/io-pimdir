---
cairn: log
change: the-crate-follows-storage-draft-08
date: 2026-11-01
---

# The spec re-vendored at pimdir d30d3c6

pimdir d30d3c6 (branch audit-2026-10-31, untagged) folds the gaps the consumers' ports found into storage-draft-08 and sync-draft-07, no draft number moving and no schema or statement touched: Annex A.1's line ends, a person settling an item conflict (SYNC §7), STORAGE §18 below SQLite 3.31, own statement text (§4.4, §17), Android's SELinux note (§5).

spec/ re-vendored from it byte for byte (`git archive`), spec/REVISION moved. Only vectors changed: sync vectors 118 to 123 and three summaries cases (56 now) with their fixtures, and the manifest. They pass unchanged: the hub's settle (f3c468a) and the walk's line ends (b41dd5d) already did what pimdir now states. `cargo test --all-features` 794 passed, `--no-default-features --lib` 353, the pending-vector list empty.

Moved: [spec-fidelity](../spec/spec-fidelity.md) and [summaries](../spec/summaries.md) (the pinned revision and the case count).
