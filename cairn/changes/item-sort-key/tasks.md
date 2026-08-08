---
cairn: tasks
change: item-sort-key
---

# Tasks

- [x] `pimdir` SPEC: §9.3 the sort key, the §11 encoding, the §12.1 reads, the
      per-kind conventions in §13, the schema and the statements.
- [x] `sql.rs`: mirror the new and changed statements from `queries/items.sql`,
      and the column and index from `migrations/0001_init.sql`.
- [x] Add the new statements to `sql::ALL` as well as declaring them, or the
      consumers reaching the canonical SQL by name (Pimalaya Android) silently
      will not see them.
- [x] `PimdirItem`: add `sort_key`, and read it in `read_item_from_row`.
      `PimdirRetainedItem` too, so the trash view keeps the same shape.
- [x] `PimdirStore`: `list_items_page_asc` / `list_items_page_desc` taking an
      `Option<(&str, i64)>` cursor, and `set_sort_key`.
- [x] `write`: no change needed. The save is diffed rather than replace-all and
      `UPDATE_ITEM` names no `sort_key`, so an existing key is preserved by
      never being touched. `LOAD_ITEMS` therefore does not need to carry it out.
- [x] Tests: a page in each direction is total across a tie; an unknown key
      sorts last descending and first ascending; an ordinary write does not
      reset a key; a rename carries items **and bindings**.
- [x] A spec-fidelity suite: schema compared semantically through pragmas,
      statement set compared by name with substitutions listed explicitly,
      every inlined statement prepared against the inlined schema.
- [x] CHANGELOG entry (additive to the read surface, breaking on `PimdirItem`).
- [x] Fold `delta.md`; log; land.
- [ ] **Follow-up, io-replica**: carry the key on `ReplicaPlacement` and
      `ReplicaFetchedItem` so it rides the ordinary insert, and the consumers'
      restating pass can go.

# Landed alongside, in the same spec pass

- [x] `ON UPDATE CASCADE` on every foreign key onto `collections(id)` **and** on
      `bindings(collection, link_id) -> items(collection, link_id)`, which is a
      parent one level down and refuses the cascade without it.
- [x] `rename_collection` in `queries/collections.sql`, and §12 stating that
      delete-and-recreate is the destructive alternative.
- [x] io-pimdir: expose `rename_collection`, and add it to `sql::ALL`.
