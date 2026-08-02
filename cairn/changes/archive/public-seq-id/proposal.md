---
cairn: change
id: public-seq-id
status: landed
created: 2026-08-02
---

# A per-collection public id (`seq`)

## Why

The client read surface keyed everything on `link_id` (`Message-ID` / `UID`), a
long opaque string that leaked into consumer UIs — it does not fit an envelope
table and is not something a user should type. `link_id` is the right *internal*
cross-source key, but the wrong *public* one.

## What

Each item gains a `seq`: a per-collection public id, IMAP-UID-like. Follows the
pimdir SPEC §9.1:

- **Per-collection, monotonic, never reused.** A `next_seq` counter on the
  `collection` hands out `seq` on item insert and only ever increases, so each
  mailbox shows small ids and a `seq` never silently addresses a different item
  after a delete. `(collection, seq)` is unique.
- **The client key.** `list_items` returns `seq` alongside the (still exposed,
  now-internal) `link_id`; `get_item` keys on `(collection, seq)`; a new
  `seq_for_link` resolves the inverse for a consumer that just staged an add and
  wants the id it now shows under.

Schema mirrors `pimdir/migrations/0001_init.sql` (`items.seq`,
`collections.next_seq`, `items_by_seq` unique index) — a breaking schema change,
re-init required (SPEC is version 1). The sync seam is untouched: it still keys on
`link_id`; `seq` is assigned transparently on insert.
