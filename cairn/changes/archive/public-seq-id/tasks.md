---
cairn: tasks
change: public-seq-id
---

- [x] Schema: `items.seq`, `collections.next_seq`, `items_by_seq` unique index.
- [x] `BUMP_NEXT_SEQ` allocator; `insert_item` assigns `seq` from it (never reused).
- [x] Read API returns `seq`; `get_item` keys on `(collection, seq)`;
      `seq_for_link` inverse; `PimdirItem.seq`.
- [x] Tests: seq per-collection, monotonic, never reused; get/list by seq (11 green).
- [x] Build, fmt, clippy clean.
- [x] Fold delta into `cairn/spec/store.md`; write log entry.
