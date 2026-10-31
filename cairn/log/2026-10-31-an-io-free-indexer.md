---
cairn: log
change: an-io-free-indexer
date: 2026-10-31
---

# An I/O-free indexer, and the store on draft-07

The Android app owns its store in Java on the platform's SQLite and wants the index without a second SQLite in the process (cairn/changes/an-io-free-indexer). The index existed only as `PimdirIndex`, rusqlite on index.db with pimdir.db attached and a `pimdir_fold` SQL function, and `search` required `client`. pimdir moved to storage-draft-07, sync-draft-06 and search-draft-07 the same day (pimdir cairn/log/2026-10-31-the-sqlite-floor-is-a-recommendation.md). Two agents landed this, the store first, the indexer second, uncommitted.

## What landed: the store on storage-draft-07

- spec/ re-vendored from pimdir byte for byte, `backfill_target_collection` included; tests/spec_drift.rs proves it.
- Flags as rows: `placement_flag` and `binding_base_flag` with `items.flags_known` and `bindings.base_flags_known`, written with `retain_flags` and `add_flags` and read back as one space-joined text; a restatement stamps nothing. `PimdirError::Flag` (`flag-invalid`) refuses a flag the store cannot hold at the enqueue and the write and parks a queued one (tests/flag_rows.rs).
- `mail_in_reply_to` and `task_category` hold the lists, read per page as the addresses are; the summary types keep their `Vec` fields.
- Every list parameter goes through `temp.param` (`create_param` on every connection, `clear_param` and `bind_param` before the statement); no statement calls a JSON function. `codec::flags_to_json`, `flags_from_json`, `ids_to_json` and `ids_from_json` are gone.
- `queue.target_collection`, written at the enqueue, read by the queue loads, followed by `rename_queue_targets`; a `NULL` target is read from the payload and parks on a renamed id.
- `store_meta.shape`: every profile refuses a store of another shape as `PimdirError::Shape` (`store-shape`), writing nothing (tests/store_shape.rs).
- The draft-06 reconciliation: the four JSON arrays read before the rebuild and written after it in the same transaction, a `NULL` set staying unknown, each queued target backfilled through the canonical `BACKFILL_TARGET_COLLECTION` (the crate's own `UPDATE` replaced by it in this change), a flag §13 refuses refusing the reconciliation (tests/earlier_lists.rs, failing against the draft-06 code first).
- The text fold of SEARCH §5 (`search::fold::text`), the tokenizer `unicode61 remove_diacritics 0`, every indexed text and every query text term folded before FTS5 reads it; an earlier index is rebuilt.

## What landed: the indexer without I/O

- Feature split (Cargo.toml): `search` no longer requires `client` nor enables rusqlite's `functions`. src/search/ now holds `chunk`, `coverage`, `extract` (moved from src/client/search/extract.rs), `fold` (with `value`, Annex A.2's fold), `occurrence` (the horizon and the expansion, split from their statements), `query`, `refresh` (the coroutine), `tag` (the plan's types and `flags`) and `thread` (private: the header parent and the relink, split from their statements). src/client/search.rs keeps `PimdirIndex`; src/client/search/refresh.rs answers the coroutine through the canonical statements; src/client/search/tag.rs keeps `PimdirIndex::tag` and `PimdirSearchTagPlan::stage`; src/client/search/occurrence.rs and thread.rs are gone. `cargo tree --no-default-features --features search -e normal` names no SQLite.
- `search::refresh::PimdirIndexRefresh::new(now, batch)`: one batch of the feed per run, one transaction per run, a `Writes` yielded before any read its ops change the answer of. `PimdirIndex::refresh` runs it per batch until a run reports the feed exhausted or the deadline passes.
- The proposal named four yields; the fold needs to read what the index holds, so there are ten: `WantsIndexState` (`index_meta`, the store's change cursor, whether a collection moved above `store_reconciled`), `WantsReexpand`, `WantsFeed`, `WantsUnread`, `WantsHeld` (the index's placement rows and object hashes, so a body is extracted once and a placement's previous key and body decide its rethreading and re-expansion), `WantsBodies` (at most `BODIES_MAX`, 16, the bytes dropped once derived, so a consumer never holds a page of bodies), `WantsSummaries` (an unparseable body or a missing blob, the feed carrying the summary of a bodiless row or a file already), `WantsMessages` (the thread graph, read as the relink walks it), `WantsGone` and `Writes`. Fifteen ops name SEARCH §3's rows.
- `WantsFeed` answers in the crate's own types (`PimdirIndexFeedRow`, `PimdirIndexSummary`), the proposal's open item settled so; purges and renames are `WantsGone`, since what is gone leaves no row in the feed and draws no stamp.
- The relink (SEARCH §9) runs over the messages read so far, changes them in memory and answers its ops, or the messages and bodies it still needs, the run asking and running it again; its ops write the thread rows before the messages pointing to them and drop the stale ones last, so a consumer enforcing the reference never sees a dangling message.
- `pimdir_fold` is gone: `category:` reads the store's distinct `task_category` values once per query naming one (`TASK_CATEGORY_VALUES`), folds them in Rust and compares those folding to the term's as stated; a card's are folded at extraction. The `name` order already sorted in Rust and now uses `fold::value`. A folded column was not an option, the index schema being the spec's.
- `sql::search`: `LOAD_PLACEMENT` replaces `PLACEMENT_HASH` and `PLACEMENT_LINK_ID`; `LOAD_ITEM_IN_REPLY_TO`, `LOAD_ITEM_CATEGORIES` (a feed summary's lists) and `TASK_CATEGORY_VALUES` are new. `PimdirError::Refresh` (`index-refresh-broken`) carries a broken coroutine contract.
- Tests: every existing test passes unchanged in what it asserts; tests/search.rs moved its tag import to `search::tag`, and every test file using the client gained `#![cfg(feature = "client")]` (tests/search.rs and tests/search_scale.rs `all(client, search)`), so the search-only build compiles them. New: tests/search.rs `memory::a_consumer_of_its_own_shape_folds_the_canonical_index` (the vectors' fixture store folded by a consumer holding its index in maps, in batches of four, then after a rename and a horizon rolled a week later, compared row for row with the canonical index.db: placements, objects, card categories, chunks, the bodies and placements holding text, flags, occurrences, messages, threads, cursor, horizon and tzdb version); tests/search_pure.rs (a query parsed, a message extracted and a refresh answered by hand, in the search-only build); src/search/refresh.rs and src/search/thread.rs unit tests. 680 tests by default, 733 with every feature, 353 with `search` alone; the 50,000-message build takes 4.7 s.
- Capabilities moved: search ("The index refreshes without I/O", "Search compiles without the client", "The index folds text before the tokenizer", added; "A refresh folds the feed in bounded, resumable batches", "Tasks are searched by status, priority and category", modified); store ("Flags are rows, read as a set", "A message's In-Reply-To ids and a task's categories are rows", "A list parameter is bound through temp.param", "A queued action's target is a column a rename follows", "A store of another shape is refused", "A storage-draft-06 store is reconciled with its lists", added).

## Departures

- The ten yields against the proposal's four, reviewed with no one yet: tasks.md keeps the review open.
- A refresh whose deadline passed before its first batch no longer rolls the horizon, the roll now running inside the first run's transaction.
- The feed's summary fills its lists (`in_reply_to`, `categories`), which the fold reads none of, at one seek per bodiless mail or task row.

## Migration

A consumer of `client::search::extract` reads `search::extract`, of `client::search::tag` `search::tag`. An app keeping its own index uses nothing of `PimdirIndex`: it answers `PimdirIndexRefresh`'s yields from its own databases inside one transaction per run and maps each `PimdirIndexOp` to its shape's statements, as tests/search.rs's `memory` module does.
