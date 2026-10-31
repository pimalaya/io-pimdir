---
cairn: change
id: an-io-free-indexer
status: landed
created: 2026-10-31
---

# An I/O-free indexer, and the store on draft-07

## Why

The Android app owns its store in Java on the platform's `android.database.sqlite` (android cairn/changes/platform-sqlite) and wants the index without a second SQLite in the process. Today the index exists only as `PimdirIndex` (src/client/search.rs), which opens index.db through rusqlite with pimdir.db attached (:1245) and registers `pimdir_fold` as an SQL function (:1250). Two SQLite copies in one process on one file break each other's POSIX locks (sqlite.org/howtocorrupt.html §2.2.1), and Android below API 30 registers no SQL function. The `search` feature also requires `client` (Cargo.toml:35), so the extraction, which needs no store, is out of reach of a build without rusqlite.

The sync engine already has the shape the index needs: an I/O-free coroutine yielding loads and writes that `client` services with rusqlite and the app services in Java (src/coroutine.rs).

## What

**Pure search without `client`.** `search` stops requiring `client`: the query language (src/search/query.rs), extraction (src/client/search/extract.rs), tags, thread derivation and occurrence expansion move under src/search/ and compile without std I/O or rusqlite; `client` with `search` keeps `PimdirIndex`. occurrence.rs and thread.rs split their derivation from their statements.

**The refresh as a coroutine.** `PimdirIndexRefresh` folds the store's change feed into the index (SEARCH §4) and yields what it needs and what it writes, each typed, never SQL:

- `WantsIndexState`: the index's cursor, horizon and tzdb version (`index_meta`);
- `WantsFeed { since, limit }`: the changed placements and purges above a stamp, with their summaries, as the canonical feed statements answer them;
- `WantsBodies { hashes }`: the bytes of the objects to extract;
- `Writes(Vec<PimdirIndexOp>)`: put or drop an object's chunks and flags, put or drop a placement, put a summary's text, replace an item's occurrences, put a thread, set the cursor. Each op names the model's rows, and a consumer maps it to its shape's statements;
- returning the coverage it reached and whether the feed was exhausted, so a caller refreshes again or stops within its deadline.

Text reaches the ops folded (SEARCH §5 as draft-07 states it), so no consumer registers a fold function: `pimdir_fold` goes, its callers reading folded columns.

`PimdirIndex::refresh` becomes the coroutine driven over rusqlite and the canonical statements, so the reference implementation and the app run one fold, tested once.

**Queries.** The parsed `PimdirQuery` is the boundary: `PimdirIndex` keeps compiling it to the canonical shape (src/client/search/compile.rs), and another shape compiles it itself. A shape-neutral plan was set aside until a second implementation needs one.

**The store on draft-07** (pimdir cairn/changes/the-sqlite-floor-is-a-recommendation): flags, base flags, `in_reply_to` and categories through their tables, list parameters through `temp.param`, `queue.target_collection`, `store_meta.shape`; the draft-06 migration; the write ops carrying flag sets unchanged at the seam.

## Release

0.9.0 (draft-06 already breaks the public types, a8bfbf0); not bumped here.

## Open

- Settled (2026-10-31): `WantsFeed` answers in the crate's own types (`PimdirIndexFeedRow`: the placement's collection, seq, link id, object hash, flag set, deleted, retained, kind, account and stamp, with `PimdirIndexSummary`, the summary row with its lists and its address rows, on every live row holding no body and on every file), never as the canonical statements' rows; a consumer of another shape maps its rows into them.
- Settled (2026-10-31): a purge or a rename is a yield of its own, `WantsGone`, not a row of the feed: the feed pages what the store holds by stamp, and what is gone leaves no row and draws no stamp.
- No pure bm25 over a `matchinfo` blob for now: the app offers `Date` and `Name` only (the user, 2026-10-31).
