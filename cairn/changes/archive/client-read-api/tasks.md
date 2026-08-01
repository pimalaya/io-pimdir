---
cairn: tasks
change: client-read-api
---

# Tasks

- [x] `sql.rs`: add `LIST_COLLECTIONS`, `LIST_ITEMS_PAGE` (keyset, `deleted = 0`),
      `GET_ITEM`, `COUNT_ITEMS`.
- [x] `client.rs`: add `PimdirCollection` and `PimdirItem` read types (item
      surfaces `level`).
- [x] `client.rs`: add `PimdirStore::list_collections`, `list_items`, `get_item`,
      `count_items`; a shared `read_item_from_row` helper.
- [x] `lib.rs`: export the new public read types.
- [x] Tests: pagination boundary (keyset `after`), tombstone exclusion,
      `level` surfaced for a Meta-only item, `get_item` hit/miss, count.
- [x] `nix develop --command cargo build` / `cargo test`; `cargo fmt`.
- [x] Fold `delta.md` into `cairn/spec/store.md`; add `cairn/log` entry; mark
      change `landed` and archive.
