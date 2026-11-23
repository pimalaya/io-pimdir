---
cairn: tasks
change: a-queued-edit-names-what-it-changes
---

- [x] pimdir storage-draft-05 / sync-draft-05 landed (spec edit 4: `if_match`, `SetFlags` sets, the unguarded window)
- [x] Test: an `update` whose `if_match` no longer matches parks and leaves the item (tests/store_rules.rs, landed with the-store-follows-storage-draft-05)
- [x] src/codec.rs, src/client/producer.rs: `if_match` encoded, decoded and checked at the drain
- [x] Test: a flag a server added between enumeration and push survives the push: vectors/sync/02, 03, 09, 26, 27 and 42 (each failed first: "push differs", a replacement set)
- [x] src/change.rs, src/sync.rs: `SetFlags { add, remove }`; seam docs for connectors
- [x] tests/vectors_sync.rs and the queue tests: the new vectors, and the §15.2 vector 88 for an unreadable body (with the-engine-follows-sync-draft-05)
- [x] Spec: seam.md, sync.md; fold, log, CHANGELOG (breaking)
- [x] Tell neverest (connectors) and the frontends staging updates: in the wave report of 2026-10-30 (neverest ports the seam next)
