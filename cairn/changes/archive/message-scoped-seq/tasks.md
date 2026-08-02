---
cairn: tasks
change: message-scoped-seq
---

- [x] Schema: `store_meta.next_seq` (global), drop `collections.next_seq`, add
      `items_by_link` index.
- [x] `insert_item` reuses the link's existing `seq` or allocates a fresh global
      one (`SEQ_FOR_LINK_ANY` + `BUMP_NEXT_SEQ` on `store_meta`).
- [x] Test rewritten: message-scoped, store-global, same id across mailboxes, new
      id per new message, never reused (11 green).
- [x] Live: a message in INBOX+Archive shares one id; himalaya reads it from both.
- [x] Fold delta into `cairn/spec/store.md`; write log entry.
