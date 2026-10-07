---
cairn: log
change: pages-walk-the-global-order
date: 2026-10-07
---

# Pages across collections walk one store-wide order

`list_mail_page_filtered` and `search_mail` order a set of collections by `(sort_key, seq, collection)`, which `items_by_sort` cannot give, so SQLite read every live row of the set by `items_by_seq` and sorted it to return one page: 49 ms over two folders of a 50k-mail store, measured by MOA.

Implements pimdir 22f1f2c (draft-04, untagged), spec/ re-vendored: the partial index `items_by_sort_global` on `items(sort_key, seq, collection) WHERE deleted = 0`, and both statements keeping their collection test off `items_by_seq` (`+i.collection`), since without `sqlite_stat1`, which nothing here gathers, the planner still prefers it and the sort.

- Open creates the index on a store an earlier draft wrote (`RECONCILED_INDEXES`, `reconcile_indexes`, cut out of the canonical DDL, in the reconcile's transaction). A reader of a store lacking it pages by a scan and a sort, correctly.
- Capability moved: store (the mail readers' plan, reconcile on open).

## Verification

`tests/scoped_sync.rs` `a_page_across_collections_walks_the_global_order`: `EXPLAIN QUERY PLAN` of `LIST_MAIL_PAGE_FILTERED` and `SEARCH_MAIL` over two collections uses `items_by_sort_global` and no `TEMP B-TREE FOR ORDER BY`. `an_earlier_draft_store_is_reconciled_on_open` drops the index, pages through a reader without it, and finds it created by the owner's open. `cargo test --all-features` (508), `--no-default-features --features cli` (508), `--no-default-features --lib` (288), clippy `-D warnings` and `cargo fmt --check` green; `spec_drift` against the pimdir checkout.
