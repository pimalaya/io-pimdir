---
cairn: delta
change: public-seq-id
---

## ADDED Requirements

### Requirement: Items carry a per-collection public id
Each item SHALL carry a `seq`: a per-collection integer id a consumer shows and
accepts in place of the internal `link_id`. It SHALL be handed out from the
collection's `next_seq` counter on insert, which only ever increases, so `seq` is
**monotonic per collection and never reused** (IMAP-UID semantics) — a stale id
never addresses a different item after a delete. `(collection, seq)` SHALL be
unique. The client read surface SHALL expose it: `list_items` returns `seq` with
each item, `get_item` keys on `(collection, seq)`, and `seq_for_link` resolves the
inverse (`link_id` → `seq`) for a consumer that just staged an add. The sync seam
still keys on `link_id`; `seq` is assigned transparently and is never a sync key.
