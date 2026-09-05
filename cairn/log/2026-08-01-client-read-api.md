---
cairn: log
change: client-read-api
date: 2026-08-01
---

# Client read API (indexed, paginated reads)

Added a read-only query surface to `PimdirStore`, the missing half of "pimdir as
a client-usable local cache" (LOCAL_STORE_PLAN §4 / action plan M1). Until now
every statement was engine-oriented (`load_hub` loads a whole collection); a
client had no way to page, get one, or count.

New SQL (`sql.rs`): `LIST_COLLECTIONS`, `LIST_ITEMS_PAGE` (keyset on the existing
`items` PK, `deleted = 0`, ordered), `GET_ITEM`, `COUNT_ITEMS` — no schema change,
no new index. New public read types (`client.rs`): `PimdirCollection`,
`PimdirItem` (the item surfaces its `level`). New methods: `list_collections`,
`list_items(collection, after, limit)`, `get_item`, `count_items`, sharing a
`read_item_from_row` helper. Exported from `lib.rs`.

The reads are kind-agnostic (raw `meta`, string flags, opaque object hash) and
observe only — the write path is unchanged, still io-replica `ReplicaWriteOp`s
through `write` (the "generic in the data, disciplined in the writes" rule). Reads
are availability-aware: each item carries its `level`, so a caller (e.g. a
Himalaya `pimdir` backend) can render a not-yet-hydrated item as "body not
fetched" instead of treating an absent body as data loss — that UI adaptation is
the client's job, not the store's.

Verified: three new `roundtrip.rs` tests — keyset pagination boundary +
collection/kind listing + get-one hit/miss + count; `level` surfaced for a
Meta-only item with no local body; selective tombstone exclusion (a deleted item
is dropped while a live sibling remains). Full suite green (8 tests), fmt +
clippy clean.

Spec updated: `store` (ADDED: client read getters; ADDED: reads are
availability-aware).
