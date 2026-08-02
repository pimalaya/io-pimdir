---
cairn: change
change: message-scoped-seq
---

## MODIFIED Requirements

### Requirement: Items carry a message-scoped public id
Each item SHALL carry a `seq`: an integer id a consumer shows and accepts in place
of the internal `link_id`. It is a property of the **message**, not the placement:
a message filed in several mailboxes (the same `link_id`) SHALL keep the **same**
`seq` in every one, so a merged / cross-mailbox view shows it once and ids never
clash between mailboxes. The store SHALL assign a message's `seq` the first time it
inserts an item with that `link_id` (in any collection) — drawing from the
**store-global** `store_meta.next_seq` counter — and reuse it for every later
placement of the same `link_id`. The counter only ever increases, so a `seq` is
**never reused** even after the message is deleted everywhere. `(collection, seq)`
SHALL be unique (one placement per message per collection). The sync seam still
keys on `link_id`; `seq` is assigned transparently on insert and is never a sync
key.
