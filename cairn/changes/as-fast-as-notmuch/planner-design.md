# Planner design (phase 2), 2026-11-21

Written from a read of io-pimdir dc19b8d. Line numbers drift; names do not.

## 1. Today's pipeline

`search` (src/client/search.rs, `fn search`) → `compile` (resolves `linked:` ends and categories) → `Compiled::new` (src/client/search/compile.rs): `node`/`term` lift each structured term to a placement set, `member()` turning it into `CASE WHEN (akey,seq) IN (...)`; each text term fills `temp.term_match` (`matches`, embedding `match_objects`/`match_summaries`), the inserts run in `compile`. `unsorted` runs `select(sort, None)`: `with()` plus two correlated aggregates per hit and `LIMIT i64::MAX`, every hit read; `page` cuts in Rust; `hit` describes each; `coverage` runs.

Measured costs (517k Enron, pimdir compared-with-notmuch):

- 125 ms floor: `with()`'s `live` CTE, a `SELECT DISTINCT` over every live item joined to collections, every term's `member` probe per row; `count` wraps the same CTE.
- Every hit read: `unsorted` takes no limit; `found` sorts the whole set.
- Threads (13.8 s for page 1 of all mail): `threads` runs `found` over all hits, one `HIT_THREAD` per hit, groups, cuts.
- Coverage (69 ms at 3,311 collections): `list_collections`; `search_temporal` (src/client/reader.rs) loops `list_coverage` per collection (20 ms); `bind_list` (src/client.rs) binds 3,311 ids into `temp.param` (3 ms); `COUNT_UNINDEXED` (src/sql/search.rs), whose `collection IN (param)` seeks `items_by_seq` per collection instead of `items_by_changed` (41 ms); the canonical `coverage`.

## 2. Plan (src/search/plan.rs, I/O-free, no rusqlite)

- `PimdirSearchSort` and `PimdirSearchCursor` (with `compare`) move to plan.rs, re-exported where they were.
- `PimdirPlanScope { kinds, accounts, collections }` from `conjuncts()`, shared with coverage.
- `PimdirPlanNode { And, Or, Not, Leaf(usize) }` over `PimdirPlanLeaf`, one variant per predicate, dates resolved to bounds.
- `PimdirPlanEstimate`, what the client measures, each capped (`LIMIT CAP`, CAP = 100,001): text `SELECT count(*) FROM (SELECT 1 FROM object_text WHERE object_text MATCH ? LIMIT CAP)` plus `summary_text`; `from:addr` a seek on `item_address_by_address`; a flag a seek on `flag`'s key; `collection:` on `items_by_sort`; `changed:` on `items_by_changed`; a date range on `items_by_sort` per scope collection; `id:`, `thread:`, `linked:`, `is:encrypted` (`object_not_ok`) small seeks; probe-only (no estimate): `kind:` (scope), `status:`, `priority:`, `has:*`, `category:`, `@domain` (a scan), `mid:` (no index on `mail_summary.message_id`). `PimdirPlanBound { rows, capped }`. No ANALYZE statistics (audit F10).
- `PimdirPlanCosts`: per-row constants, FTS5 defaults (set row about 1 µs, materialised match 0.15 µs, text probe 6 µs, seek 1 µs, head 1 µs a collection); the Android bridge passes FTS4's (86 µs a probe).
- `PimdirPlanDriver { Set(leaf), Union(leaves), Walk { budget, fallback: Option<leaf> }, Scope }`; per leaf `PimdirPlanAccess { Materialize, Probe }`, a text term materialised when `bound × 0.15 µs < candidates × 6 µs`.
- `PimdirWalk`: an I/O-free threshold merge (a coroutine, as the indexer, so the bridge reuses it): the collections' head keys, ordered; one step per collection with the page's limit-th key as a floor; stop when the next head falls below it.

Choice:

- Count: the smallest positive conjunct set (`Union` for an `or` of sets); none positive: `Scope`.
- Page newest/oldest first: `Set` when the bound is at most 20 × limit or cheaper than heads plus expected probes; else `Walk` with a budget of about `B × set_row / probe_row` rows, falling back to `Set` past it. Dated collections walked descending (ascending for oldest-first), then undated ascending.
- `title`, `sender`: the whole set, folded in Rust as today's `page`.
- `relevance`: a `Set` over the positive text terms' `term_match`, `min(rank)`.
- Negations are always probes (`NOT EXISTS`); an `or` is a probe unless every operand has a set.

SQL fragments:

- `REP(y)`: `NOT EXISTS (SELECT 1 FROM store.items j JOIN store.collections c2 ON c2.id=j.collection WHERE j.seq=y.seq AND j.deleted=0 AND j.id<>y.id AND c2.account IS c.account AND S(c2) AND (j.sort_key>y.sort_key OR (j.sort_key=y.sort_key AND j.collection<y.collection)))`: a walk emits a hit at its representative placement alone.
- `S(c)`: the scope inline (`c.kind='message/rfc822'`), no `temp.param`.
- `HAS(h,P)`: `EXISTS` over the hit's live placements in its account (an `items_by_seq_global` seek) with `P(j)`; probes keep today's semantics, over every placement of the account.
- `T(n,h)`: `EXISTS (SELECT 1 FROM temp.term_match t WHERE t.term=n AND t.akey=h.akey AND t.seq=h.seq)`.
- `TXT(e,y)`: `HAS(y, EXISTS(SELECT 1 FROM placement p JOIN object o ON o.hash=p.hash WHERE p.collection=j.collection AND p.seq=j.seq AND EXISTS(SELECT 1 FROM object_text WHERE object_text MATCH e AND rowid=o.id)) OR EXISTS(… summary_text MATCH e AND rowid=p.id))`.
- `SETPAGE(d)`: `SELECT h.akey,h.seq,max(j.sort_key) k,max(c.kind IN ('text/vcard','application/octet-stream')) u FROM d h JOIN store.items j ON j.seq=h.seq AND j.deleted=0 JOIN store.collections c ON c.id=j.collection AND ifnull(c.account,'')=h.akey AND S(c) [WHERE probes] GROUP BY 1,2 HAVING AFTER(:cursor) ORDER BY u, CASE WHEN u=0 THEN k END DESC, CASE WHEN u=1 THEN k END, h.seq, h.akey LIMIT :limit+1`.
- `STEP`: `SELECT y.sort_key,y.seq FROM store.items y INDEXED BY items_by_sort WHERE y.collection=?1 AND y.deleted=0 AND (?2 IS NULL OR y.sort_key<?2 OR (y.sort_key=?2 AND (y.seq>?3 OR (y.seq=?3 AND ?4>?5)))) AND y.sort_key>=?6 AND REP(y) [AND probes] ORDER BY y.sort_key DESC, y.seq LIMIT ?7` (?4 the collection's account, ?6 the threshold).
- `HEADS`: `SELECT c.id, ifnull(c.account,''), (SELECT max(sort_key) FROM store.items WHERE collection=c.id AND deleted=0 AND sort_key<=coalesce(?1,x'')) FROM store.collections c WHERE S(c)`.

The bench's queries:

- `kiwi` (bound 35, Set): count `SELECT count(*) FROM (SELECT DISTINCT akey,seq FROM temp.term_match WHERE term=0) h WHERE HAS(h,1)`; page `SETPAGE(...)`.
- `from:jeff.dasovich@enron.com` (11k, Set): `d = SELECT DISTINCT ifnull(c.account,'') akey, i.seq FROM store.item_address a JOIN store.items i ON i.id=a.item AND i.deleted=0 JOIN store.collections c ON c.id=i.collection WHERE a.address=?1 AND a.role='from'`; larger bounds walk with the probe `EXISTS(SELECT 1 FROM store.item_address WHERE item=j.id AND role='from' AND address=?)`.
- `is:unread` (no positive set): page by Walk with `NOT HAS(y, EXISTS(SELECT 1 FROM flag f WHERE f.flag='\Seen' AND f.collection=j.collection AND f.seq=j.seq))`; count by `Scope`, O(store) until F6 (system flags read from `marks`, `items_unread`).
- `kind:mail` page 1 newest first: `HEADS` with `S = c.kind='message/rfc822'`, then `STEP` over a few dozen collections; count `Scope` with `REP(y)`.
- `kind:mail --threads` page 1: Walk in batches of 2 × limit, `HIT_THREAD` per hit, grouping until limit + 1 threads; per thread shown `THREAD_MEMBERS`, each member tested by `SELECT 1 … WHERE j.seq=?1 AND akey AND S(c) [AND probes] LIMIT 1`, then `THREAD_SPAN`; later pages skip a thread whose least hit place is at or before the cursor.
- `california not gas`: page Walk with `TXT(california) AND NOT TXT(gas)`; count with both materialised, `... WHERE NOT T(1,h) AND HAS(h,1)`.
- `folder:dasovich-j/inbox`: one collection, `REP` drops out; page one `STEP`; count on `items_by_sort`.
- `from:dasovich date:2001-01-01..2001-12-31 california`: estimates `{from}:"dasovich"` 11k, california 40k (materialised), date capped; page `SETPAGE(term 0)` with `T(1,h)` and a date `HAVING`.

## 3. SEARCH §8.2 delta

Both paged forms fit §8.2's words but one: a hit is placed by the greatest key of every live placement (compile.rs `select`), and a scoped walk cannot see a placement outside the scope. F1's own item: "a hit stands by the greatest sort key of its live placements in the query's collections", keeping "greatest" for oldest first too, with a vector: an item in two collections under different keys, scoped by `collection:`, both orders, paged one by one.

## 4. Coverage

Per query: skip `COUNT_UNINDEXED` when `behind` is 0, else `SELECT collection, count(*) FROM store.items INDEXED BY items_by_changed WHERE changed>:cursor AND deleted=0 GROUP BY collection`; one `SOURCES` statement (`SELECT collection, covered_since, covered_until, covered_at FROM store.sources ... ORDER BY collection, source`) in place of a `list_coverage` per collection; the scope bound once. Within one `PimdirIndex`, a cache keyed on `PRAGMA data_version` of both databases (per collection, stale set from `collections_by_changed` and `items_by_changed` past the cached stamps; full reload when `purges` moved or `store_change` went down). Gap: a `sources` INSERT or DELETE stamps nothing (only an UPDATE trigger), so `SOURCES` is re-read whenever `store.data_version` moves.

## 5. Order of work and risks

1. Coverage. 2. Sort and Cursor to plan.rs. 3. plan.rs with unit tests (choice tables, the walk against a brute-force merge). 4. compile.rs as the renderer (`live`, `with`, `select` deleted). 5. search.rs: `search` and `count` on the plan, `threads` on the walk; a `#[doc(hidden)]` forced driver so tests/search.rs runs every vector under Set, Walk and Scope.

Likely to break: the ordered cases (mixed dated and undated, the `(home,1)`/`(work,1)` tie broken by account, the mixed-direction keyset), the limit-1 paging test in tests/search.rs, item 12 filed twice (dedup by `REP`), `id:` with digits, `person:`, the date leaf's union of occurrences and undated kinds. Likely to miss the gate still: the counts of all mail, of `enron` and of `is:unread` (a distinct count over 517k); follow-ups a per-placement lead bit or per-collection counters.
