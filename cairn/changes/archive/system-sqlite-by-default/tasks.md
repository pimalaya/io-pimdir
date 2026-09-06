---
cairn: tasks
change: system-sqlite-by-default
---

# Tasks

- [x] Cargo.toml: `vendored = ["rusqlite?/bundled"]`, `rusqlite` without `bundled`.
- [x] shell.nix: `sqlite` in `buildInputs` and on `LD_LIBRARY_PATH`.
- [x] Move src/{hub,mutate,rekey,sync,upgrade}/tests.rs into their modules.
- [x] Build and test both ways: default (system) and `--features vendored`.
- [x] Fold the delta into cairn/spec/store.md; log; land.
