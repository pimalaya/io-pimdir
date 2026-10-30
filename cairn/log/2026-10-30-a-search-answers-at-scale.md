---
cairn: log
change: a-search-answers-at-scale
date: 2026-10-30
---

# A search answers at scale

himalaya measured its `pimdir search` on 50,000 summary-only messages: 454 s to build the index, about 25 s for a page of `is:unread` and 24 s for its count. Both costs grew with the square of the store.

## What landed

- tests/search_scale.rs (ignored): 50,000 summary-only messages through the owner, the index built, a page of 50 and a count per query, the compiled statement's query plan with `PIMDIR_PLAN` set; `PIMDIR_SCALE` sets the size.
- src/client/search/compile.rs: every term's membership is `CASE WHEN (akey, seq) IN (...) THEN 1 ELSE 0 END` (`member`). A bare row-valued `IN` may be `NULL` as far as SQLite knows, so under `NOT` it walked the whole set for each hit outside it. A page reads hits' titles for the `name` order alone.
- src/client/search.rs: extraction reads one item's summary through `get_mail` and its siblings by `(collection, seq)` and its addresses through `load_addresses`, seeks; it read `load_*_summaries` and `load_addresses_by_link` filtered by `:links`, whose `:links IS NULL OR link_id IN (...)` plans as a scan of the collection, once per item.
- Capabilities moved: search ("A search answers at the scale of a mailbox", added).

## Numbers (release build, this crate's bench)

| | before | after |
| --- | --- | --- |
| build, 5,000 | 4.3 s | 0.46 s |
| build, 50,000 | 454 s (himalaya) | 4.8 s |
| `is:unread` count, 5,000 | 259 ms | 2.4 ms |
| `is:unread` page, 5,000 | 279 ms | 21 ms |
| `is:unread` count, 50,000 | 24 s (himalaya) | 25 ms |
| `is:unread` page, 50,000 | 25 s (himalaya) | 117 ms |
| `date:2026-08-01..` page, count, 50,000 | about 0.5 s (himalaya) | 112 ms, 53 ms |

No canonical schema or statement changed.
