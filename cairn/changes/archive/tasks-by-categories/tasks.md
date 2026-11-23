---
cairn: tasks
change: tasks-by-categories
---

# Tasks

Waits on pimdir cairn/changes/tasks-by-categories (landed 2026-10-28).

- [x] Re-vendor spec/: delete_unbound_item.sql (its comment); the vectors are read from the sibling checkout
- [x] tests/search.rs answers the four view queries (every_search_vector_answers, no code change)
- [x] `PimdirStore::put_item` on an unbound collection: summary and references in the same transaction, refused on a synced collection; a_collection_a_source_syncs_is_refused fails without the refusal
- [x] `PimdirStore::ensure_time_collection` and `client::TIME_COLLECTION`
- [x] tests/unbound_items.rs: write, restate, delete a VEVENT in the time collection with no refcount drift; no source syncs it; mail, files and undeclared refused; a card
- [x] delta.md, fold into cairn/spec/store.md, log, CHANGELOG
