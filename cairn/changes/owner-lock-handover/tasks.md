---
cairn: tasks
change: owner-lock-handover
---

- [x] Check the premise: a second description in one process conflicts on `flock`
- [x] Reproduce the spurious `Owned` from four threads over the raw registry
- [x] Move the `File` into the registry, counted, closed inside the mutex
- [x] The existing owner-lock properties still hold, untouched
