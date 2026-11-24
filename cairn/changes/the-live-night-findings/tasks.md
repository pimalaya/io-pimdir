---
cairn: tasks
change: the-live-night-findings
---

# Tasks

- [x] P1: reproduce from the scratch store's parked rows; find where the fourth key repeats the third. Reproduced 2026-11-24 on a copy (row 53 retried and drained, parked again); `PimdirMutate` read the target for `k` and `dup:k#\1k` only, so it claimed `dup:dup:k#\1k#\1k` unread (cairn/log/2026-11-24-a-fourth-copy-takes-a-key-of-its-own.md).
- [x] P2: mint the next free key. pimdir vector 215 failing first, then the fix: the target is read again for each key minted over a held one. "From the original link id" is left as R1 below.
- [ ] R1 (owner ruling): should a copy of a minted item mint from the original link id? Today it mints from its own key, which is free and deterministic; recovering the original means parsing a key STORAGE §9 calls opaque, ambiguous for a key minted over a real handle holding `#`. Recommendation: keep it.
