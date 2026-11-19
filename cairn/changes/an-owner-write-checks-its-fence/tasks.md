---
cairn: tasks
change: an-owner-write-checks-its-fence
---

- [x] The owner rules (2026-11-19: after v1, a storage-v1 minor adding a `store_meta` column; Android exempt)
- [ ] pimdir: STORAGE §4.3 (`store_meta` owner epoch), §8 (the fence, the lease), a statement drawing and one checking it, a scenario in checks/invariants.sh
- [ ] src/client/lock.rs: draw the epoch when the role is taken
- [ ] src/client/write.rs and every owner write: check it inside the transaction, `PimdirError` for a stale owner
- [ ] Test: a second process taking the role makes the first one's next write fail with nothing written
- [ ] Spec: store.md; fold, log, CHANGELOG
