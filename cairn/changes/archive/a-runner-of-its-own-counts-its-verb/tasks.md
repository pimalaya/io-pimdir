---
cairn: tasks
change: a-runner-of-its-own-counts-its-verb
---

- [x] `client::PimdirRunning` owning its lock share, `PimdirSourceStore::running` in src/client/run.rs, `run` through it
- [x] tests/gc.rs `a_running_guard_keeps_the_collector_out`: the collector on two handles, the eviction and the release refuse while the guard is held, the collector reclaims once it drops
- [x] cairn/spec/store.md folded, log, CHANGELOG
- [x] neverest holds the guard in `run_verb_paged` (neverest cairn/log/2026-11-19-the-drive-loop-counts-its-verb.md)
- [ ] The Android app's own runner holds the guard (android's to do, if it resumes coroutines itself)
