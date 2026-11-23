---
cairn: change
id: a-search-answers-at-scale
status: landed
created: 2026-10-30
---

# A search answers at scale

## Why

himalaya measured its `pimdir search` on 50,000 summary-only messages (release build, pimdir alone): building the index took 454 s; `is:unread` took about 25 s per page of 50 and 24 s per count, the count being io-pimdir's query alone; `date:`, `from:` and `subject:` answered in about 0.5 s. Both grow with the square of the store.

## What

Measured with tests/search_scale.rs (ignored; 50,000 messages in one collection, a third read, a seventh flagged) and EXPLAIN QUERY PLAN of each compiled statement, which the bench prints with `PIMDIR_PLAN` set:

- `is:unread` compiles to `NOT ((akey, seq) IN (SELECT ... FROM flag ...))`. SQLite cannot prove the row-valued set holds no `NULL`, so under `NOT`, where `NULL` and false part ways, it walks every row of the set for each hit the set does not hold, to tell a partial `NULL` match from none. Every term's membership is now the test of a `CASE`, where `NULL` is false, one probe of the set (src/client/search/compile.rs `member`). At 5,000 messages the count went from 259 ms to 2.4 ms.
- A refresh read each bodiless item's summary and addresses through `load_mail_summaries` (and siblings) and `load_addresses_by_link` filtered by `:links`: `collection = :collection AND (:links IS NULL OR link_id IN (...))` plans as a scan of the whole collection (EXPLAIN: `SEARCH mail_summary USING INDEX sqlite_autoindex_mail_summary_1 (collection=?)`), once per item. It now reads one item through the canonical `get_mail` (and siblings) by `(collection, seq)` and `load_addresses` by `(collection, link_id)`, both seeks. At 5,000 messages the build went from 4.3 s to 0.46 s.
- A page computed every hit's title, six summary tables joined per hit, before ordering and cutting: only the `name` order reads it now.

No canonical schema or statement changes: the fixes use statements SEARCH and STORAGE already define.
