---
cairn: tasks
change: a-listing-proves-what-it-drops
---

- [x] pimdir sync-draft-05 landed (spec edit 3); thresholds and the delta-page rule read from it
- [x] Test: a round dropping 20 of 20 bound members drops none and reports it: vectors/sync/77 (failed first: "events differ", then rows), 78 for `accept_absences`
- [x] src/sync.rs: the guard on a round's last page; `PimdirSyncReport::parked` (no event: the vectors report the park beside the events)
- [x] Test: a paged delta whose page carries the checkpoint and `last` false (failed first: the checkpoint landed)
- [x] src/sync.rs: the delta page rule (refused, as SYNC §4 makes a delta one page)
- [x] src/client.rs: `delete_collection` refuses queued rows (landed with the-store-follows-storage-draft-05)
- [x] What a connector owes before deleting a server collection, documented on the seam
- [x] tests/vectors_sync.rs: vectors 74 to 79 and 89 (rekey park), the two-source case 74 to 76
- [x] Spec: sync.md, rekey.md, seam.md; fold, log, CHANGELOG
