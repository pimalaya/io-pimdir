---
cairn: change
id: a-query-only-index
status: landed
created: 2026-10-30
---

# A query-only index

## Why

The plan of 2026-10-30 makes the store's owner (neverest) the indexer: it builds and refreshes index.db at sync time, and frontends (himalaya's `pimdir search`, plan-himalaya-pimdir-search.md) only query. Today `PimdirIndex::open` creates, rebuilds and refreshes, always read-write with WAL (src/client/search.rs:1107), against SEARCH §3:44 (query clients open index.db read-only).

The audit also found (report-io-pimdir.md, "Search"; review-io-pimdir.md B5, B6, S17 to S22):

- `collection:` and `folder:` also match a collection's display name (compile.rs:370, client/search.rs:1046), against STORAGE.md:383 (a name is a label, never an address);
- a rename stays unreconciled after a deadline-cut refresh (client/search.rs:504-511), the fix needing an `index_meta` column;
- `occurring_between` misses zero-length occurrences at a range's start (compile.rs:384-401, the spec's SQL);
- `has:attachment` ignores the `attachment` field (compile.rs:303-306);
- `horizon_left` prose against queries.json:30;
- undocumented extensions (compile.rs:395, :549; cli/search.rs:533) and 16 own statements outside queries/search;
- stringly outputs (`kind: String`, `Status(String)`).

pimdir search-draft-05 (2026-10-30, should-fix-before-the-freeze) fixed `occurring_between`, `has:attachment` and `horizon_left` in the spec, and stated `id:` with digits as a hit set (the `seq` and every item whose link id it is) and `linked:` reading a role only when it is one; this crate vendors that text already.

## What

- `PimdirIndex::open_query(dir)`: index.db opened read-only (no create, no journal-mode write, no lock), never built, rebuilt nor refreshed. A missing index is `PimdirError::IndexMissing` (`index-missing`), one at another schema version, tokenizer or build `PimdirError::IndexOutdated` (`index-outdated`); `refresh` on such a handle is `PimdirError::IndexQueryOnly` (`index-query-only`). `PimdirIndex::open` stays the indexer's open (build, rebuild, refresh).
- Every answer says how far the index lags the store: `PimdirSearchLocal::behind`, the store's change stamps above the index cursor (its head minus the cursor, store-wide), beside the scoped `unindexed` count. SEARCH.md states no such field: reported to pimdir as a gap.
- `collection:` and `folder:` match collection ids only, in the compiled query and in the coverage's scope.
- A deadline-cut pass no longer loses a rename: the collection stamp the last completed pass reconciled from is kept in the crate's own `index_build` table (`reconciled`, build 2, so an older index is rebuilt), the analogue of `store_purges`. SEARCH's `index_meta` has no such column and SEARCH §4 compares the collection stamp with a cursor the batches advance: reported to pimdir as a gap (an `index_meta` column, an index schema major).
- `id:` and `linked:` with a value of digits alone name the item of that `seq` and every item whose link id it is (SEARCH §8); `linked:x-:id` reads `x-` as part of the id, `x-` naming no role.
- The query client re-chunks nothing: an index this crate's `index_build` marker does not name is rebuilt by the indexer and refused by the query open, so stored chunk offsets are always this extraction's (SEARCH §6).
