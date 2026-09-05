---
cairn: tasks
change: store-write-path
---

- [x] `stage_blobs` before `BEGIN` in `write` and `write_rekeyed`
- [x] `OBJECT_EXISTS` per file in the collector, added to the format's statements
- [x] `RETURNING` on both purges, retiring the two read-first statements
- [x] `LIST_GARBAGE_OBJECTS` back to the canonical `<= 0`
- [x] The whole suite passes untouched: none of this changes behaviour
