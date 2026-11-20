---
cairn: tasks
change: the-crate-follows-draft-15
---

# Tasks

- [x] Re-vendor spec/ from the pimdir working tree at the draft-15 bump
- [x] Constants and version tests at draft-15
- [x] Search no longer experimental (D3); the binary outside the 1.0 contract (D4)
- [x] cairn/spec folded, the log written, CHANGELOG.md
- [x] spec/REVISION names the pimdir commit carrying the bump (476c6a0, then 5371b73 with the generator); pimdir pins this crate in checks/io-pimdir.rev (1460f07)
- [x] spec/generator/ vendored and checked by tests/spec_drift.rs; no dev-dependency declared yet
- [x] The change-feed refresh vectors (vectors/search/refresh.json), run by tests/search.rs
- [ ] The later draft-15 lines of pimdir's freeze plan: D2, the query language
