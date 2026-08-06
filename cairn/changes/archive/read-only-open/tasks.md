---
cairn: tasks
change: read-only-open
---

# Tasks

- [x] Add `PimdirStore::open_read_only` (SQLITE_OPEN_READ_ONLY, no migration, exact-version check)
- [x] Test: reads an owner-created store, refuses a missing one, and its write path errors
- [x] Fold into cairn/spec/store.md and log
