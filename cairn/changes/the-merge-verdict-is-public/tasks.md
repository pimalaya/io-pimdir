---
cairn: tasks
change: the-merge-verdict-is-public
---

- [x] `PimdirHubItem::reconcile` and the public verdict types in src/hub.rs, `absorb` through them
- [x] tests/hub_reconcile.rs: each policy's loser and side, an unread body diverging whatever the policy, a fill settling the conflict recorded without it; the hub's own tests pass unchanged
- [x] cairn/spec/hub.md folded, log, CHANGELOG
- [ ] The Android app drops `lost` and `diverges` from rust/src/offline/merge.rs for `reconcile` (android's to do)
