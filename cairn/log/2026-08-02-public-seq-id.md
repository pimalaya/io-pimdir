---
cairn: log
change: public-seq-id
landed: 2026-08-02
---

# A per-collection public id (`seq`)

The client read surface keyed on `link_id` (a long `Message-ID`/`UID` string),
which leaked into consumer UIs. Added `seq`, a per-collection public id.

- Schema (mirrors `pimdir/migrations/0001_init.sql`): `items.seq INTEGER NOT
  NULL`, `collections.next_seq INTEGER NOT NULL DEFAULT 1`, `items_by_seq` unique
  index on `(collection, seq)`. Breaking schema change (SPEC v1) — re-init.
- `BUMP_NEXT_SEQ` (`UPDATE … RETURNING`) hands out the collection's next id on
  each `insert_item`; the counter only rises, so a `seq` is never reused even
  after a delete (IMAP-UID semantics).
- Read API: `PimdirItem.seq`; `list_items`/`get_item` return it; `get_item` keys
  on `(collection, seq)`; new `seq_for_link` is the inverse for an add.
- The sync seam is untouched — it still keys on `link_id`; `seq` is assigned
  transparently on insert.

Tests: `public_seq_is_per_collection_monotonic_and_never_reused` (INBOX numbers
1,2,3; Sent numbers independently from 1; a dropped id is not reused) plus the
read-surface tests updated to key on `seq` (11 green). Downstream verified:
neverest writes seq-bearing stores; himalaya shows short ids (1..N) and
reads/edits by them.

Spec updated: `store` (ADDED "Items carry a per-collection public id"; MODIFIED
the read-surface requirement to expose `seq` and key `get_item` on it).
