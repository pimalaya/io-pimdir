# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

The store follows pimdir's storage-draft-18, the index search-draft-18 and the sync sync-draft-16 (the owner decisions of 2026-11-25). Drafts are exact: a store an earlier draft wrote is refused, never migrated, and an index of an earlier draft is rebuilt by the indexer.

### Added

- **BREAKING**: Added `PimdirMutateError::Unsynced` (`target-unsynced`) and `PimdirMutate::target_synced`: a copy or a move into a collection no source syncs, holding no binding nor checkpoint there, is refused, staging nothing in either collection, and a queued `move` or `copy` there parks (pimdir SYNC §7, STORAGE §15.3). An exhaustive `match` on the enum needs the arm.
- Added `PimdirMutation::create_target` to the public API, the collection a runner of its own loads and writes as the source a copy's or a move's create is staged under.

### Changed

- **BREAKING**: A copy or a move into a collection no source syncs is refused (above), where it used to stage a pending create under the run's source, binding that source to a collection it never synced: a target is synced, by a checkpoint or a binding, before anything is copied or moved into it.
- The overlay folds a pending `move` into a collection no source syncs as nothing, its item left where it is, as the drain parks it (pimdir STORAGE §15.4).
- **BREAKING**: Changed the index to pimdir's search-draft-18 (`sql::search::FORMAT_VERSION`, `FORMAT_DRAFT` 18): an index of search-draft-17 is rebuilt by the indexer and refused by a query client.
- **BREAKING**: Added the field `point` to `PimdirQueryDateRange`, whether a date value is one point rather than a range; a struct literal states it.
- Changed `FIELDS` to follow pimdir SEARCH §8's table, `flag:` before `is:`.

### Fixed

- Fixed a copy or a move whose run's source does not sync its target staging the create under that source, which never pushed it, while another source of the target was offered the item, pushed it and left the stray pending create standing: the create is now staged under the source an `add` into the target runs as, the owner's own where it syncs the target, else the first source syncing it, with no origin of the run's source, the target loaded and written as that source in the transaction of the move's tombstone, through `PimdirSourceStore::mutate` and the drain alike, so that source's own tombstone of the identity is the one revived (pimdir SYNC §7, STORAGE §15.2; vectors 227, 228 and 230).
- Fixed a date range from a relative amount to itself (`date:2w..2w`) answering the span from that instant to now: a relative amount runs to now only as a single point, a side of a range spanning its instant alone, so the range is empty (pimdir SEARCH §8.1).
- Fixed `linked:` naming a role and no id (`linked:sender:`) read as the link id `sender:`: it is refused `invalid-value` (pimdir SEARCH §8).

## [0.20.4] - 2026-11-25

### Fixed

- Fixed a store closed and opened again in one process reported owned by another process (`PimdirError::Owned`) when another thread was spawning a child: the child holds a copy of the locked description until its `exec`, and the release only closed this process's descriptor. A release now unlocks the description before closing it, the owner lock and the staging lock alike (seen as himalaya's `pimdir_queue` test failing in CI, 5 of 80 parallel runs).

## [0.20.3] - 2026-11-24

### Added

- Added `PimdirHub::release_shared`: the body a member under a bare `Message-ID` shares with a member minted from it that took it in the batch absorbed, released from the bare one as a release does, for the next upgrade to fetch (pimdir SYNC §6, STORAGE §14).
- Added `sql::LIST_MINTED_LINKS`, the keys a collection holds minted from the link ids of the `links` slot.

### Fixed

- Fixed two messages stating one `Message-ID` in one mail collection holding one body (neverest's live test on Stalwart, a move into a mailbox holding another message stating the moved one's `Message-ID`, of one size): the target's own member had linked the moved message's body by agreement on subject, date, sender and size when it was listed, and kept it once the move landed, the moved body shown and acted on as the other message's. A `Full` upgrade now links no body by agreement into a collection holding another member stating the placement's hint, only under an equal server object id, the `Handles` load reading each such member beside the one named; and the write giving a member under a minted key the body the bare one holds releases it from the bare one, which the next upgrade fetches: one download per such member, on every server (pimdir sync-draft-15 and storage-draft-17 edited in place).

## [0.20.2] - 2026-11-24

### Fixed

- Fixed a lock that fails for another reason than another holder being reported as one: the owner, staging and probe locks answered `Owned`, `Staging` or `Busy` on any `flock` error, telling the user another process holds the store; such an error is now `PimdirError::Io` with its cause.

## [0.20.1] - 2026-11-24

### Fixed

- Fixed `search` failing to compile for a non-Unix target without `indexer` (Windows releases of himalaya, cardamum and calendula): the hit columns read their file with `Read`, imported only under `indexer`.

## [0.20.0] - 2026-11-24

The store follows pimdir's storage-draft-17, the index search-draft-17 and the sync sync-draft-15. Drafts are exact: a store an earlier draft wrote is refused, never migrated, so a user starts over, moving the store directory aside and syncing anew (`neverest` re-creates it); an index of an earlier draft is rebuilt by the indexer. Every CLI sharing a store moves to this release together.

The features split by domain (cairn/changes/archive/the-query-split-by-domain): `client` the store, `search` the index's read side over it, `mail`, `contact` and `calendar` each domain's extraction, `indexer` the refresh over the three without `client`, `vendored` SQLite bundled. As the consumers take them: neverest `search` and `indexer`, the one CLI building and refreshing the index; himalaya `mail`, with `client` and `search` for its pimdir backend; cardamum `contact`, with `client`, `search` and `contact` for its pimdir backend; calendula `client`, `search` and `calendar`; the `pimdir` binary (`cli`) `search` and `indexer`; the Android app `indexer` alone. A client refreshing an index in its tests only takes `indexer` as a dev-dependency.

### Added

- Added `PimdirSourceStore::settled_collections`: the collections where a run of the source that lists nothing new derives nothing, with the coverage each checkpoint serves, so a caller whose source states no change may leave them unsynced (`pimbench sync` does under `PIMBENCH_SKIP=1`: the 517k no-op from 75 to 21.5 ms).
- **BREAKING**: Added `PimdirWriteOp::LandPlacement`: a relocation stating the handle it gave its member (`PimdirPushResult::assigned` on an accepted relocating `Remove`, an IMAP `COPYUID`) lands the destination's create on it in the write recording the outcome, so a later listing never lands it on another message stating the same `Message-ID` (pimdir SYNC §4, sync-draft-15 edited in place). An exhaustive `match` on the enum needs the arm.
- Added the server object id (pimdir SYNC §4, §6; STORAGE `bindings.server_object_id`, storage-draft-15 edited in place): `PimdirRemoteMeta::server_object_id` and `with_server_object_id` for a connector stating RFC 8474's `EMAILID`, Gmail's `X-GM-MSGID` or JMAP's `Email/id`, recorded on the member's binding; a `Full` upgrade links no body from a holder whose binding of the same source states another id, where the agreement on subject, date, sender and size alone linked two different messages sharing a `Message-ID`.
- **BREAKING**: Added the field `server_object_id` to `PimdirRemoteMeta`, `PimdirPlacement`, `PimdirBinding` and `PimdirHeldObject`; a struct literal of any of them states it (`None` where the source states no id).
- Added the features `mail`, `contact` and `calendar`, each compiling its domain's extraction alone (and `contact` the card matcher), and `indexer`, the I/O-free index refresh over the three, without `client`.
- Added `search::mail::PimdirMail`, `PimdirQuery::match_mails` and `PimdirQuery::mail_terms`: a query matched over messages held in memory, read with their bodies or from their header alone, as an index holding them answers, under `mail` without `client`.
- Added the `mid:` field, matching a message's `Message-ID` whole, with or without its angle brackets (`PimdirQueryPredicate::Mid`, pimdir SEARCH §8 in search-draft-16).
- Added instants to `date:` and `when:` (`2026-08-01T10:00`, with seconds, `Z` or an offset; `PimdirQueryDate::Instant`), read in the query's zone when they state no offset, and the relative units `min`, `h` and `mo` (pimdir SEARCH §8.1).
- Added `PimdirQueryDateRange::bounds`, a range's instants in Unix seconds, for a client translating a query to a server.
- Added `PimdirQueryPredicate::name()`, the name a client's capability table keys a term on.
- Added `PimdirSearchSort::as_str`, `parse` and `ALL`.
- Added an item's properties, local state keyed by its public id that no source carries, a local label (`pimdir/label`) among it (pimdir STORAGE §4.3, storage-draft-15 edited in place at pimdir daec31a): the module `property` (`PimdirProperty`, `PimdirPropertyHolder`, `PimdirPropertyChange`, the bounds, `is_key`, `is_writable_key`, `is_value`, `fold`, `same` and `change`), the reads `PimdirReader::list_item_properties`, `list_property_values`, `list_property_holders` and `list_value_holders`, the overlay `PimdirReader::item_properties` (§15.4), and the owner's own writes `PimdirStore::add_property` and `remove_property`.
- **BREAKING**: Added `PimdirAction::SetProperty`, the `set-property` action (pimdir STORAGE §15.3): one key, its removes applied before its adds, a label compared folded, applied by the drain as no source with a receipt, parked when no live row of its collection holds the item or past 64 values under the key. An exhaustive `match` on the enum needs the arm.
- **BREAKING**: Added `PimdirError::Property` (`property-invalid`): a property the owner cannot write, past a bound or under a key `pimdir/` keeps.
- Added the `pimdir property list|add|remove` verbs, staging a `set-property` through the queue as `tag` stages its flags.
- Added the query fields `label:` and `property:` (pimdir SEARCH §8, search-draft-16 edited in place at pimdir daec31a), answered from the store's `item_property`, never the index, for every account's hit of the item: `label:` an item's label, both sides folded as Annex A.2 folds `fn`, `property:<key>=<value>` a value byte for byte. An empty value, one past 1,024 bytes or holding NUL, a `property:` value without `=` and a key out of `<namespace>/<name>` or past 64 bytes are refused `invalid-value`.
- **BREAKING**: Added `PimdirQueryPredicate::Label` and `PimdirQueryPredicate::Property` and `PimdirPlanEstimate::Property`, which an exhaustive `match` names.
- Added `search::plan`, the I/O-free query planner (scope, estimates, drivers, `PimdirWalk`), `PimdirSearchSort::is_dated`, and `PimdirSearchCursor::{compare, dated, is_undated, sort_key, seq, account}`; `PimdirSearchSort` and `PimdirSearchCursor` now build without `client`.
- Added the refusal `reserved-form` (`PimdirQueryErrorCode::ReservedForm`, pimdir SEARCH §8.3): a text or a value beginning with `~`, a text or a value of two characters or more beginning with `/` and a value beginning with `{` are kept for stemming, regular expressions and subqueries, where they answered as plain words before. Quote them to search the characters.
- Added `pimdir version`, the one probe of the JSON contract: `formatVersion`, io-pimdir's major version (`0` below 1.0), beside the crate's version and the STORAGE, SEARCH and SYNC drafts the build implements (`pimdir-version`). No other payload carries the contract's version.
- Added `codec::keywords_to_text`, a flag set's other flags as `items.keywords` and `bindings.base_keywords` hold them (pimdir STORAGE §13, storage-draft-17): every flag but the five system flags, sorted bytewise and joined by one space, `''` for none and for an unknown set.
- Added the statements `sql::UPSERT_SUMMARY`, `DELETE_SUMMARY`, `LOAD_SUMMARIES`, `LOAD_SUMMARIES_BY_LINK`, `REPLACE_VALUES`, `INSERT_VALUE`, `LOAD_VALUES`, `LOAD_VALUES_BY_LINK` and `LOAD_VALUES_BY_COLLECTION`, which write and read the one `summary` table and the one `summary_value` table whatever the kind, and `sql::COLLECTION_HOLDS_ACTIONS`, which `delete_collection` asks before it deletes.
- Added `sql::search` under `indexer` without `client`: the index's migrations, statements, `FORMAT_VERSION` and `all()`, text and constants alone, for an indexer binding its own SQLite with no rusqlite in its build.
- Added `summary::DERIVATION`, the Annex A revision recorded in `store_meta.derivation`, public without `client` for an owner holding its own SQLite (pimdir audit-2026-11-12 A5).
- Added `PimdirSubmit::to_payload`, the `v: 1` payload `to_action` wraps, which a rename writes back with its `copy` renamed.

### Changed

- Changed a query client built with `search` and no `indexer` to read the hit columns (index.hits) the indexer writes, as one built with both does: counts, dated pages and threads skip the index.db path the cache exists to spare (himalaya N6: `pimdir count california` at 517k from 80 to 14 ms in its own build, measured by its lane). Writing the cache stays under `indexer`.
- Changed `PimdirReader::accounts_of` to read the accounts and kinds alone (`sql::ACCOUNTS_AND_KINDS`, added), no coverage join: the answer is unchanged, about 4 ms less at 3,313 collections on every command resolving an account (himalaya N7).
- Changed a load naming no link to read no hub: a sync of a collection owing nothing runs seven statements fewer (the 517k no-op from 95 to 75 ms with every folder synced). The answer of every load is unchanged.
- **BREAKING**: Changed the store schema to pimdir's storage-draft-17 (pimdir f295ed7) and the index to search-draft-17 (`sql::FORMAT_VERSION`, `sql::search::FORMAT_VERSION`): a store of storage-draft-16 or earlier is refused as an earlier engine's, moved aside and synced anew; an index of search-draft-16 or earlier is rebuilt by the indexer and refused by a query client until then. In the store, the flags but the five system flags are the columns `items.keywords` and `bindings.base_keywords` in place of the tables `placement_flag` and `binding_base_flag`; `mail_summary`, `contact_summary`, `event_summary`, `task_summary`, `journal_summary` and `file_summary` are one `summary` table keyed by `items.id`, its `component` naming the kind; `item_address`, `mail_in_reply_to` and `task_category` are one `summary_value(item, role, position, value, name)` table; `queue.collection` is `ON DELETE RESTRICT` in place of the `collections_keep_queued` trigger. In the index, the `chunk` and `thread` tables are gone: a snippet cuts its chunks from the body, and `message.thread` names a thread whose span is read from the store. A consumer reading these tables in its own SQL follows them; the public types and the reader statements' names and answered columns do not move (the readers alias `title AS subject`, `uid AS message_id`, `card_kind AS kind`).
- **BREAKING**: Changed the statements, the index schema and the test generator to be this crate's own: pimdir binds none of them since pimdir 692af79. They stay under spec/queries/, spec/migrations/search/ and spec/generator/, edited here, and `io_pimdir::sql` keeps exposing every statement by name; only spec/migrations/storage/, spec/vectors/ and spec/payloads/ are pimdir's, byte for byte at spec/REVISION.
- Changed `PimdirIndexOp::PutThread` and `PimdirIndexOp::DropThread` to be yielded for a consumer whose index keeps a row per thread; `PimdirIndex::refresh` applies neither. `PimdirIndexObject::chunks` is likewise for a consumer storing chunks.
- Changed `PimdirIndex::search` and `threads` to answer in one read transaction, a page's threads read in two statements (`sql::search::HIT_THREADS`, `THREADS_HELD`, replacing `HIT_THREAD`), a snippet cut only up to its first chunk naming a word, and a hit the hit columns proved matched in a body answered so without a probe: on the 1M Enron bench pages and threads 0.3 to 2.6 ms faster, every answer unchanged.
- Changed the refresh to patch index.hits where it moved placements, byte for byte what a whole read gives, rather than reading every placement again (a 300-message refresh at 517k from 617 to 189 ms), and `PimdirStore::ensure_collection` to write nothing when the collection already holds the kind.
- Changed a build of the index from empty to hold FTS5's merges back and merge each full-text table into one segment at its end, runs sharing a transaction up to 16,384 feed rows: a 517k rebuild from 121.8 to 92.2 s and 22.9 to 16.5 GB written, the index at rest one segment a table.
- Changed the `pimdir` binary's fixed cost of a query: SQLite's memory statistics off, a query-only index left to the process exit, `count` and `search` parsed by a two-verb root, store and index judged by their meta row in one read, coverage counts skipped when no row could feed them (`sql::search::COVERAGE_HELD`): about 0.5 ms off every query.

- Changed a `subject:` or `from:` word or prefix answered off the hit columns to read `pimdir_fields`, posting lists of the titles' and From headers' words the refresh keeps in index.db beside the canonical tables, in place of FTS5's column filter over every body holding the word: an index built before it is rebuilt by the indexer, and answered from by a query client meanwhile.
- **BREAKING**: The store schema is storage-draft-16 (pimdir STORAGE §3, three partial indexes on `sources`): a store of storage-draft-15, which 0.19 wrote, is refused as an earlier engine's. A query no conjunct narrows reads its temporal coverage through the canonical `covered_all` (pimdir SEARCH §8.2), three index seeks, where the crate's own roll read every source (about 0.4 ms off each unscoped count and page on a store of 6,622 sources).
- Changed counts, dated pages and threads to read a cache of flat columns, index.hits, which the refresh writes beside index.db and maps read-only (`rustix` gains its `mm` feature): text terms read the full-text index's rowids with no placement join, system flags and dates one pass over the records or a test per hit, and a page sorts only its candidates. On the 1M Enron bench counts go from 24 to 210 ms to 4 to 55, and pages and threads of selective terms from 35 to 124 ms to 10 to 16, every answer unchanged. Any query the columns cannot answer, or an index another writer moved, reads index.db as before.
- Changed the count of a query of text terms alone on a current index to match its terms together, one FTS5 expression over the bodies and one over the summaries, evaluating per placement only the hits whose placements name two or more documents, which the index now records (`split`, pimdir SEARCH §3, §4 and §8.2, search-draft-16 edited in place): `power california` from 43 to 13 ms and `california not gas` from 47 to 17 on the 517k Enron bench. An index built before this is rebuilt on its next refresh, its schema text differing.
- Changed the `pimdir` binary's JSON output into a contract (pimdir the-freeze-plan D4, amended 2026-11-21): what `--json` prints for `search`, `count`, `show`, `thread`, `tag`, `item list`, `item show`, `collection list`, `queue list`, `queue status`, `store info` and `version`, and the failure object `pimdir-error`, is stable within a major, a minor only adding keys, and values to vocabularies declared open. Each payload's schema is pinned by tests/json_contract.rs against a committed copy under tests/json_contract/. The commands, the flags, the text output and the operator verbs stay outside io-pimdir's 1.0 contract, as `pimdir --help` and README.md say.

- Changed a dated page of an `and` of text terms on a current index to read its hits from the count's bitmaps when the terms match at most 40,000 documents together, joined to the store by one statement, rather than walking the store's order (`power california` at 1M messages from 337 to 112 ms).
- Changed a write batch's bodies, on Linux, to be written first, each one's writeback started by `sync_file_range`, then all flushed and only then moved onto their names (`PimdirBodyFlush`, `PimdirSourceStore::body_flush`, `Deferred` by default on Linux, `Each` elsewhere), the order a body reaches the disk before a row names it unchanged (STORAGE §5): a 517k message import's sync from 572 s to 305 s.
- **BREAKING**: Changed the vendored storage-draft-15 to pimdir's the-owing-rows-are-marked, edited in place: `bindings.owes` and `items.lacks`, kept by triggers, let a delta's `list_owing_bindings` and `list_lacking_items` seek what is owed (a sync finding nothing at 517k messages from 0.40 to 0.12 s; an import 4 to 10% more CPU). A store of the previous text is refused as damaged and rebuilt by a fresh sync.
- Changed `PimdirIndex::count` of system flags alone (`is:unread`, `is:read`, `is:flagged`, `tag:` naming a system flag, with `and`, `or`, `not`) over a scope of kinds and accounts to pass once over the scope's placements off `items_by_sort` when that costs less than reading its cheapest flag set (`is:unread` at 517k messages, two thirds unread, from 126 to 69 ms; `not is:unread` from 462 to 57). A store mostly read keeps reading its few unread rows.
- **BREAKING**: Added `PimdirPlanCosts::flags_row`, `unread_row` and `marked_row`, and `PimdirPlan::scans_flags`: a struct literal of `PimdirPlanCosts` needs the fields.
- Changed `PimdirIndex::count` to run in one read transaction, its coverage included, and, while the index holds the store as it stands, to seek no live placement in the store for a hit a text term drives; a query of text terms alone over every collection is counted off the full-text index, each term's placements marked once in a bitmap per account and the bitmaps combined (`enron` at 517k messages from 506 to 107 ms, `power california` from 125 to 44). The vendored copy follows pimdir c5f1e85, which pins a negated text term beside text terms alone.
- **BREAKING**: Changed `search` to read the index alone: it implies `client` and compiles `PimdirIndex::open_query`, the queries, `item` and `tag`, no domain; `PimdirIndex::open`, `refresh` and `erase` need `search` and `indexer`. A reader takes the domains it shows (`item` and the snippets read a body of a domain left out from its summary). The card matcher moves to `contact`, the refresh coroutine to `indexer`.
- **BREAKING**: Changed `search::query` and `search::coverage` to compile with no feature.
- **BREAKING**: Moved `search::occurrence::FLOATING_ZONE` to `search::coverage::FLOATING_ZONE`.
- **BREAKING**: Changed `PimdirSearchSort` to SEARCH §8.2's five orders, `NewestFirst` (was `Date`, the default), `OldestFirst`, `Title` (was `Name`), `Sender` and `Relevance`, and the CLI's `--sort` to `newest-first`, `oldest-first`, `title`, `sender` and `relevance`.
- **BREAKING**: Added the variants `PimdirQueryPredicate::Mid`, `PimdirQueryDate::Instant` and `PimdirQueryDateUnit::{Minute, Hour, Month}`, which an exhaustive match names.
- **BREAKING**: Removed `#[non_exhaustive]` from `PimdirQueryErrorCode`, so a match names every code.
- Changed `search`, `count` and threaded pages to run on the planner: a query reads what its driving set or its page needs, never every live item (performance-audit F1). A hit's key is read among its placements in the query's collections, which only differs where one item's placements hold different keys.
- Changed the planner to costs measured on FTS5 under SQLite 3.51 over the 517k Enron messages (`PimdirPlanCosts`, with `source_row` and `source_probe`), and added `PimdirPlanAccess::Sources`: a text term probed per candidate reads the ids of the bodies and summaries it matches off the full-text index once (`temp.term_source`), each probe then a seek rather than a full-text query (two words and a negation paged in 90 and 72 ms, 1,346 and 1,403 before). A match read into a set no longer computes `bm25` unless the order is `relevance`, a page's hits tell what matched from the same ids, and `PimdirPlan::page` weighs a walk by the scope's `PimdirPlanExtent` and the sets' shares of it.
- Changed `PimdirWalk` to step the collection of the best next row from a queue ordered by key, a collection first asked for 8 rows, and a set page and a date set to read the store from their hits and collections, never by a scan.
- Changed a count of a query whose every term the scope implies (`kind:mail`, `account:`) to read each live placement of the scope once and mark its hit in a bitmap per account (all mail counted in 41 ms, 193 before), and a coverage over every collection to read no collection's row (`sql::search::ROLL_COVERAGE_ALL`, `COVERAGE_ALL`).
- Changed the store's and the index's schema check to compare each object with the text of the migrations' statements, no database built for it: 2.4 ms less on every open.
- Changed `PimdirSync::WRITE_CHUNK` from 1,024 to 4,096, so a page of 1,000 new members lands in one transaction.
- Changed the index refresh to write a batch's full-text rows after its other writes, so FTS5 writes one segment a batch instead of one a body, to take each message's parent from its extraction instead of reading its body again, to fold ASCII text without normalising it, to relink without a scan per message, and to drop summary text only for a placement held bodiless: 342 to 186 µs a message on 40,000 Enron messages, the index rows unchanged.
- Changed `PimdirIndexRefreshOptions::default().batch` from 256 to 4,096 rows and an indexer's WAL checkpoint from 1,000 to 16,384 pages: a first build writes about a quarter of the bytes, for up to 47 MB of memory and 64 MB of WAL.
- Changed a search answer's coverage, and `PimdirReader::search_coverage`'s temporal axis, to read once per answer: the scope's ids bound once, the sources rolled up in one statement instead of `list_coverage` per collection, the unindexed count seeking `items_by_changed` and skipped while the index is not behind, a repeated answer on one `PimdirIndex` served until index.db or the store moves (performance-audit F8: 69 ms per answer at 3,311 collections).
- Changed coverage to count a body under an extraction status it does not know as `unparseable` (pimdir SEARCH §6, an open vocabulary).
- Changed every connection to turn on SQLite's query planner stability guarantee, so a cached statement is no longer compiled again each time it is bound to another collection: a sync that finds nothing in 3,311 collections from 2.5 s of CPU to 1.05 s, and search pages up to 60% faster.
- Changed a delta to leave out `SetCheckpoint` when its checkpoint is the one the source holds, and a sync to ask for no last write when it has none to apply: a sync that finds nothing takes no transaction and no `fsync`.
- Changed `PimdirSourceStore::load` to read one snapshot in one read transaction, and a `Handles` selection naming no handle to read the sync state and the refusals alone.
- **BREAKING**: Changed the index to search-draft-16 (`sql::search::FORMAT_VERSION`): an index of search-draft-15 is rebuilt by the indexer and refused by a query client until then.
- **BREAKING**: Changed the vendored storage-draft-15 to pimdir daec31a's, edited in place: the table `item_property`, its index and triggers, and `evict_before` keeping an item holding a property. A store created by an earlier build of this draft lacks the table and is refused by the schema check; drafts are exact, so it is moved aside and synced again.
- **BREAKING**: Changed the vendored storage-draft-15 and search-draft-16 to pimdir 8b1523d's, edited in place: an object is keyed by an integer, `objects.id`, the hash unique on the object row alone, and every pointer at a body (`items.object`, was `object_hash`, `items.conflict_object`, `bindings.base_object`, `conflict_object` and `shared_object`, `queue.object`, was `object_hash`) holds that id; index.db's `placement.object` (was `hash`) holds the index's `object.id`. A store of the previous storage-draft-15 text is refused by every profile as damaged (`store-corrupt`, naming `objects`), never migrated: drafts are exact, so it is moved aside and synced again; an index of the previous text is rebuilt by the indexer and refused by a query client (`index-outdated`) until then. The statements keep the hash as their parameters and answered columns, so no Rust signature moved; a consumer running its own SQL on the store or the index joins by the ids.
- Changed the crate's own statements to join by the integer keys: the planner's text-term probes join a placement to `object_text` by its `object`, reading no `object` row, as `match_objects` does: on the 517k Enron store, counts of `enron`, `california` and `power california` in 516, 58 and 121 ms (947, 106 and 220 before), and the diagnoses (`REFCOUNT_DRIFT`, `LIVE_BYTES`, `COUNT_RETAINED_BEFORE`, `COUNT_BODILESS`) join pointers to objects by id. `pimdir check` names a pointer at no object row by its id, `#` before it (`DANGLING_ITEM_OBJECTS`, `DANGLING_QUEUE_OBJECTS`), no row holding its hash.
- **BREAKING**: Changed the vendored search-draft-16 to pimdir e2b7f49's, edited in place: the index's `flag` table holds keywords alone and refuses the five system flags by a `CHECK`, so the refresh writes a placement's flags but `\Seen`, `\Answered`, `\Flagged`, `\Draft` and `\Deleted` (`PimdirIndexOp::ReplaceFlags`). An index of the previous text is rebuilt by the indexer and refused by a query client (`index-outdated`) until then; a consumer of its own shape folding `ReplaceFlags` into SQL of its own writes no system flag either.
- Changed `is:read`, `is:unread`, `is:flagged`, `is:answered`, `is:draft`, and `tag:` or `flag:` naming a system flag, to read the store's `items.marks` as it stands, never the index (pimdir SEARCH §8, §8.2, `unread` and `marked`): while the index lags, a query's flag terms answer the store after its last write and its text terms the store as last indexed. `is:unread` drives from `items_unread` instead of passing over every live item, each hit still probed, so a message read in one of its collections stays out: on the 518k Enron store the unread count from 572 to 116 ms, while `is:flagged`, which walks `items_by_sort` where it sought the index's `flag` table, goes from 28 to 45 ms counted and 15 to 31 ms paged.
- **BREAKING**: Added `PimdirPlanEstimate::Marked` and `PimdirPlanEstimate::Unread`, the estimates of a system flag's set (`Flag` is a keyword's now), and `PimdirPlan::are_exact`. An exhaustive `match` on the enum needs the arms.

### Removed

- **BREAKING**: Removed the flag statements, the flags being columns of their row: `sql::ADD_FLAGS`, `RETAIN_FLAGS`, `ADD_BASE_FLAGS` and `RETAIN_BASE_FLAGS`.
- **BREAKING**: Removed the per-kind summary and list statements, merged into the statements Added names: `sql::UPSERT_MAIL_SUMMARY`, `UPSERT_CONTACT_SUMMARY`, `UPSERT_EVENT_SUMMARY`, `UPSERT_TASK_SUMMARY`, `UPSERT_JOURNAL_SUMMARY`, `UPSERT_FILE_SUMMARY` (to `UPSERT_SUMMARY`); `DELETE_EVENT_SUMMARY`, `DELETE_TASK_SUMMARY`, `DELETE_JOURNAL_SUMMARY` (to `DELETE_SUMMARY`); `LOAD_MAIL_SUMMARIES`, `LOAD_CONTACT_SUMMARIES`, `LOAD_EVENT_SUMMARIES`, `LOAD_TASK_SUMMARIES`, `LOAD_JOURNAL_SUMMARIES`, `LOAD_FILE_SUMMARIES` (to `LOAD_SUMMARIES`) and their six `_BY_LINK` twins (to `LOAD_SUMMARIES_BY_LINK`); `REPLACE_ADDRESSES`, `REPLACE_IN_REPLY_TO`, `REPLACE_CATEGORIES` (to `REPLACE_VALUES`); `INSERT_ADDRESS`, `INSERT_IN_REPLY_TO`, `INSERT_CATEGORY` (to `INSERT_VALUE`); `LOAD_ADDRESSES_BY_COLLECTION` (to `LOAD_VALUES_BY_COLLECTION`); `LOAD_ADDRESSES_BY_LINK`, `LOAD_IN_REPLY_TO_BY_LINK`, `LOAD_CATEGORIES_BY_LINK` (to `LOAD_VALUES_BY_LINK`); `LOAD_ADDRESSES`, `LOAD_IN_REPLY_TO`, `LOAD_CATEGORIES` (to `LOAD_VALUES`).
- **BREAKING**: Removed `sql::DELIVER_BINDING`, `LINK_SENDERS_OF_BY_LINK`, `LIST_GARBAGE_OBJECTS` and `SET_STORE_VERSION`, which nothing ran.
- **BREAKING**: Removed `sql::search::CHUNKS_OF`, `INSERT_CHUNK`, `UPSERT_THREAD`, `DELETE_THREAD` and `SET_EXTRACTION`, with the index's `chunk` and `thread` tables; an index whose extraction revisions differ is rebuilt, never restated in place.
- **BREAKING**: Removed `sql::search::HIT_THREAD`, a page's threads read by `HIT_THREADS` and `THREADS_HELD`.
- **BREAKING**: Removed the `Send` implementation of `PimdirIndex`, which holds the hit columns it maps (index.hits): an index is used on the thread that opened it. A build with `search` and no `indexer` maps them since this release (himalaya N6), so it loses `Send` too.

### Fixed

- Fixed the Windows build, which failed on an untyped inode in the search cache's identity (src/client/search/hits.rs).
- Fixed the builds of `mail`, `contact`, `calendar` and `indexer` without `client`, which warned of a hub projection only the client uses.
- Fixed a rekey whose listing states a member's flags unknown setting its binding's base to the unknown set, which dropped a flag change still owed instead of pushing it on the next sync: the base keeps its own set (pimdir SYNC §8, vector 201). A silent loss.
- Fixed a rekey whose listing states no revision clearing a binding's conflict, leaving the local edit to be pushed and the diverging body collectable: the conflict is carried as it is, its revision and body kept (pimdir SYNC §8, vector 202). A silent loss.
- **BREAKING**: Fixed a rekey naming its members otherwise than a page (pimdir SYNC §8, sync-draft-15 edited in place): a delivered create a member resolves to is landed as a page lands it, its provisional handle dropped `Superseded`, its staged flags kept over a base of the listed flags and its staged body, and `Created` reported, where it was carried with the flags merged and nothing reported (vector 203); a create the store offers the source joins the member stating its key, where a second item was minted (vector 204); a pending create lands on a member stating its key and agreeing with its summary, reported `Created`, where the member was minted apart and the add pushed again, a second copy on the server (vector 205, rule D10 of cairn/changes/what-the-consumers-need); a member disagreeing with it is minted apart and reported `Disagreed` (vector 206). A rekey's report carries these events, and a listing carrying a body for a landed create stores it.
- **BREAKING**: Fixed a rekey settling or reviving a conflicted tombstone: it is carried with its conflict, listed at its base revision or at its conflict's, a delete settling none, where the divergence was lost and a `Remove` derived, or the tombstone revived clean (pimdir SYNC §8, vectors 207, 208). A tombstone holding a local edit met by a remote edit is carried as a `Conflict` at the fetched revision holding the listed body, as a page records it, where it was revived as a pull and the edit kept only by an item retained apart (vector 209); a rekey's report carries `Overruled` for a body its write kept apart.
- **BREAKING**: Fixed a rekey whose listing repeats a handle mixing its entries, the first's flags under the last's identity, summary and sort key, the old item dropped `Deleted`: the first entry is kept whole, as a page keeps it (pimdir SYNC §4, §8, vector 210). A rekey now stores a body its listing carries, inline or persisted, as a page does: an arrival's and a pull's become the item's body and the base's at `Full`, where they were dropped and the member left at `Meta` (vectors 211, 212).
- Fixed the queue's drain of a `copy` or a `move` into a collection where another source binds the identity minting a second item under a `dup:` key, where `store.mutate` joins that source's item: the drain reads its target as a `Links` load does (pimdir SYNC §7, STORAGE §14).
- Fixed `PimdirReader::pending_actions`, `list_pending_actions` and `PimdirProducer::pending_actions` failing a whole listing on one row whose payload does not decode: the row is listed as `PimdirAction::Unknown`, its kind and payload verbatim, beside the rows that decode, until the drain parks it (STORAGE §15.4); `codec::payload_outcome` tells it malformed and `codec::action_from_payload` names its error. `pimdir queue list` prints that error on the row, and no longer reports its collection in `undecodable`.
- Fixed a flag change staged before a move being undone when the relocation states its member's handle (an IMAP `UID MOVE` answering `COPYUID`): the landed create takes as its base the flags the member carries, so the staged set is pushed instead of pulled back over (pimdir sync-draft-15, vector 214). `PimdirWriteOp::LandPlacement` gains `flags`.
- Fixed a fourth copy of one message into a collection being refused and parked, given the third copy's key: `PimdirMutate` reads the target again for each key minted over a held one until a free one (pimdir sync-draft-15, vector 215).
- Fixed a copy, a move or an add into a collection where the same source holds the message as a tombstone not pushed yet being parked for good ("a binding pins one handle"): the tombstone is revived under the handle it is bound to and its delete withdrawn; one holding another message has the copy or the add minted beside it (pimdir sync-draft-15, vectors 216 to 219).
- Fixed a send queued with a copy to file failing "FOREIGN KEY constraint failed" across a rename of that collection an earlier run made: `PimdirStore::rename_collection` rewrites the `copy` of every queued `submit`, pending or parked (pimdir STORAGE §14).
- Fixed a parked move, copy or superseding add retried after a rename of its target parking again on the old id: `PimdirStore::rename_collection` follows `queue.target_collection` on parked rows too (pimdir STORAGE §14).
- Fixed a flag change staged on a delivered create, or a copy reviving a tombstoned one, giving its base the item's body, which left the create on its provisional handle when a relocation stated its member's handle (pimdir sync-draft-15, vectors 220 and 221).

## [0.19.1] - 2026-11-21

### Fixed

- Fixed a rekey carrying a local edit in conflict onto its new handle with the summary and sort key of the server's body, which the item does not hold (pimdir SYNC §8).
- Fixed an upgrade reading the base under a local edit giving the item the summary and sort key of the base, the edit staying its body (pimdir SYNC §6).
- Fixed a bodiless pull recorded as an item divergence, written twice in one batch, giving the item the summary and sort key of the unread body it kept apart (pimdir SYNC §5, §9).

## [0.19.0] - 2026-11-20

### Added

- **BREAKING**: Added `PimdirRekeyReport::events`, `Added` for a member a rekey names that no old placement resolves to (pimdir SYNC §5, §8); the report is no longer `Copy`.
- **BREAKING**: Added `PimdirLoaded::folded`, the placements owing a change another source folded into their item, answered by `PimdirHub::folded`: a storage building `PimdirLoaded` by a struct literal needs the field.
- Added `PimdirStore::set_collection_parent` and `PimdirStore::set_collection_presentation`, writing a collection's parent (its account's hierarchy kept a forest) and its color, description and sort order through pimdir's new owner statements, with `PimdirError::Parent` (`collection-parent`) and `PimdirError::Undeclared` (`collection-undeclared`).
- **BREAKING**: Added `PimdirWritten::adopted` and `PimdirWritten::refused`, the handles whose body a write's policy took over the item's or kept apart (pimdir SYNC §9), which a sync reports as a pull or not: a storage building `PimdirWritten` by a struct literal needs both fields.
- Added `PimdirReader::pending_arrivals`, the pending moves and copies other collections queued into a collection (pimdir STORAGE §15.4, `load_pending_arrivals`).
- Added `PimdirEnqueued::link_id`, the key an `add` was checked under, stated or derived from its body in hand, so a producer states none and reports the key the store will hold.
- **BREAKING**: Added `PimdirError::HashKeyMismatch` and `PimdirMutateError::HashKeyMismatch` (`hash-key-mismatch`): an `add` stating a `hash:` key its body's bytes do not give is refused, and a drain parks one (pimdir STORAGE §15.3, SYNC §7).
- **BREAKING**: Added `PimdirConflictPolicy::KeepLocal`, the policy of a source the store is authoritative over (pimdir SYNC §5, sync-draft-15): a change made on its remote alone is a divergence settled for the store, its body kept and reported `Overruled` and the store's pushed over it, never pulled. An exhaustive `match` on the enum needs the arm.
- **BREAKING**: Added pimdir storage-draft-15, sync-draft-15 and search-draft-15, vendored from the pimdir working tree of 2026-11-20: a store or a sync state of draft-14 is refused, never migrated, and an index of draft-14 is rebuilt.

### Changed

- Changed the search extractor revision to `io-pimdir 3`, on ical-rs 0.7.3: an index built before is extracted again.
- Changed the documentation of the search layer: it follows the crate's versioning like the store and the engine, SEARCH being released together with STORAGE and SYNC.
- Changed `pimdir --help` and README.md to state that the `pimdir` binary is outside the crate's 1.0 contract: its commands, flags and output may change in any release.

### Fixed

- Fixed a keyless `add` being queued with no key: the key it was checked under is written into its row, so a second `add` of the same key meets it as a pending duplicate.
- Fixed a sync offering another source an item under a `hash:` key, which asserts no identity (pimdir SYNC §9).
- Fixed an item keeping its body under a divergence taking the incoming body's summary and sort key (pimdir SYNC §9).
- Fixed a kept body's retained item carrying the winning item's sort key instead of the one its own body gives (pimdir SYNC §9, STORAGE Annex A).
- Fixed a server edit meeting a change another source folded being recorded as the source's own conflict: it is the collection's divergence, settled by its policy (pimdir SYNC §5, §9).
- Fixed a sync recording a contact or event joining an item another source binds, listed without its body, as a conflict of its binding: it is the item's conflict, which the upgrade reading a body equal to the item's now clears, as it no longer leaves two equal bodies in conflict (pimdir SYNC §6, §9).
- Fixed a sync reporting a member joining an item another source binds as `Created` and settling a diverging body it carried by the source's own policy: it is `Added`, its body folded into the item by the collection's policy (pimdir SYNC §6, §9).
- Fixed a sync reporting `ContentChanged` for a pull whose body the collection's `prefer-existing` kept apart, the item keeping its own (pimdir SYNC §5).
- Fixed `PimdirIndex::refresh` keeping a body indexed after its last placement left it or moved off it, and a thread's id and bounds after a member's sort key moved: a refreshed index is the one a rebuild writes (`PimdirIndexOp::DropUnplaced`, pimdir SEARCH §4, §9).
- Fixed `PimdirReader::with_pending` folding rows the owner parks: an action on an item no source binds where the owner settles it alone, a move into a collection of no declared kind, and a row the capability backstop parks fold into nothing, and a folded update carries the summary and sort key of its new body, paged by that key.
- Fixed `enqueue_with_body` queueing a UID-less contact or event under the `hash:` key its caller derived before the `X-PIMDIR-` properties were taken out, a key naming bytes the store never held: it is refused, and stating no key answers the right one.
- Fixed an upgrade reading the body a base named at its revision, while the item's conflict held its diverging body, fast-forwarding the item to it: the conflict cleared and both the source's edit and the diverging body were lost. The base takes the body and the item and its conflict stand.
- Fixed `PimdirIndex::search`, `threads` and `thread` failing the whole page when one hit's blob cannot be read: that hit lists without a snippet.
- Fixed `PimdirReader::with_pending` showing an item a pending move or copy brings in without the rows queued on it in its source before: it arrives with them folded in.
- Fixed `PimdirIndex::refresh` failing the whole run on a body it cannot read: its placement is recorded missing and tried again.
- Fixed an upgrade settling an item conflict recorded before its incoming body was read, under the collection's `prefer-incoming`, giving the item the fetched body but keeping the summary, sort key and address rows of the body it held (pimdir SYNC §9, STORAGE §9.3, Annex A).

## [0.18.1] - 2026-11-19

### Added

- Added `PimdirSourceStore::running`, the in-flight guard `run` holds, for a runner of its own driving a coroutine through `service`, so the collector refuses while its verb runs.
- Added `client::PimdirRunning`, that guard, owning its share of the lock.

## [0.18.0] - 2026-11-19

### Added

- Added `client::blobs::PimdirBlobMove`, the move of a written body onto its name that never replaces a file, for an app writing bodies itself.
- Added `client::blobs::PimdirBlobFiled`, what a `PimdirBlobMove` did.
- Added `PimdirStore::purge_items`, purging chosen retained items in one transaction.
- Added `hub::PimdirHubItem::reconcile`, the cross-source merge of one upsert, for a storage applying upserts itself.
- Added `hub::PimdirHubVerdict`, what `reconcile` decided: a divergence, the body kept, the body lost.
- Added `hub::PimdirHubLoser`, a body a policy did not adopt.
- Added `hub::PimdirHubLoserSide`, which body it was.
- Added `collection::media_type`, the bare media type of a collection kind, which every rule of the crate keyed on a kind now reads.
- Added `PimdirReader::accounts_of`, the accounts holding collections of a media type, an undeclared collection counting for none.
- Added `PimdirReader::overlay_quiet`, whether no pending action changes what a read of a collection shows.
- Added `PimdirQuery::scoped`, a query narrowed to a kind, an account and a collection as top-level conjuncts, which scope its coverage.
- Added `PimdirSearchCoverage::unread_counted`, the bodies out of sight that count against an answer, by the rule `is_complete` reads.

### Changed

- Changed the sync conformance suite to the sync vectors' format 8 (an object's `row`), and vendored the vector stating a file under its name with no row (pimdir sync vector 172).
- Changed the blob move to `renameat2` with `RENAME_NOREPLACE` (`renameatx_np` with `RENAME_EXCL` on Apple), falling back to `link(2)`, then on Unix to an exclusive create and copy where the link is denied.

  A store an Android app writes, its SELinux policy denying `link(2)`, is filed by the same move as any other.

### Fixed

- Fixed `pimdir` panicking (exit 101, nothing printed) on a `--log-file` it cannot open: the error is reported on the output contract, exit 1.

## [0.17.0] - 2026-11-18

### Added

- **BREAKING**: Added pimdir storage-draft-14, sync-draft-14 and search-draft-14, vendored at pimdir da7e01a.

  A store of an earlier draft is refused by every profile (`store-version`), and one whose sync state an earlier engine wrote too (`sync-version`), never migrated; an index of an earlier draft is rebuilt. `sql::FORMAT_VERSION`, `sync::SYNC_VERSION` and `sql::search::FORMAT_VERSION` name them (`sql::search::FORMAT_DRAFT` is 14), and `init_store_meta` and `init_index_meta` changed their text.
- Added the binding of a refused offered create: a create the store offers a source binding nothing of the item, refused for good, is bound as that source's pending create with its refusal, so it is held by its key, reported every run and read by the receipt, where it was pushed and refused every run.
- Added `version::PimdirFormatRefusal` and `version::PimdirFormatOrder`: the one refusal of a store of another format version, ordering it against the program's by pimdir STORAGE §4.2 (none, then drafts by number, then releases by major) and saying which is the earlier and what that asks, the program naming itself. `PimdirError::Version` reads so for "this program".
- Added `PimdirProducer::enqueue_user_body`, queueing a body whole, for a resource the user wrote as it stands or whose `X-PIMDIR-` properties they asked for.
- Added `PimdirLinkId::landed_hint` and `PimdirLinkId::is_minted`: the one recognition of a minted key, which `is_landable` and the engine's lookup of the create a listed hint lands now read.
- Added `search::card::PimdirCard`, `PimdirQuery::match_cards` and `PimdirQuery::store_terms`: a query matched over cards held in memory, as an index holding those cards answers it (SEARCH §11's contact cases), under `search` without `client`; a term only a store answers refuses the match (`PimdirCardMatchError`).

### Changed

- Changed the documentation of `PimdirPushOutcome` to the refusal exceptions of pimdir draft-14: a `401`, `407` or `423` and a quota exceeded are no refusal, left unreported.
- **BREAKING**: Changed `PimdirProducer::enqueue_with_body` to remove every `X-PIMDIR-` property from a calendar or address-book body before queueing it (pimdir STORAGE §18), as `enqueue_foreign_body` did, which is removed. A caller queueing an instruction the user asked for moves to `enqueue_user_body`.
- Changed the message of `PimdirError::Version` (`store-version`) to say which store is the earlier: a draft store under a release, or no format version, reads as an earlier sync engine's, drained then set aside; a later one asks for a later program.

### Fixed

- Fixed a recurrence rule whose `BYxxx` parts fill every period stalling the indexer for as many instances as its `COUNT` names (22 s for one event): an expansion walks at most 10,000,000 instances of a rule from its `DTSTART` (`search::occurrence::INSTANCES_MAX`, pimdir SEARCH §7 in search-draft-14), cut at the first not walked, so such a series writes no occurrence past that instant.
- Fixed a search page sorting every hit of its query, and an overlaid read reading the conflicts of every item with a pending update, remove or move in the collection rather than of the items it shows.

## [0.16.0] - 2026-11-17

### Added

- Added `PimdirSearchCursor`, the place of the last hit a search page answered, written and read back as a token (`token`, `from_token`).
- Added `PimdirLinkId::minted_from`, the keys a minted key may be minted from.
- Added `PimdirHub::immutable`, set by the store for a `message/rfc822` collection.
- Added `PimdirMutateError::MoveInPlace` (`move-in-place`).

### Changed

- **BREAKING**: Changed search paging from an offset to a cursor: `PimdirSearchOptions::after` replaces `offset`, and `PimdirSearchOutput::next` and `PimdirSearchThreadsOutput::next` name where the next page starts.

  A page now starts after the last hit the page before it answered, so a hit arriving or leaving in between neither repeats nor hides another. The `pimdir search` `nextPage` and `--page-token` carry the cursor's token; a token printed before is refused as `page-token-invalid`.
- Changed `EXTRACTOR` to `io-pimdir 2`: every search index another revision built is extracted again once.
- Changed `pimdir show`, and `search`, `count`, `tag` and `thread` given `--refresh-timeout 0s`, to answer from the index as it stands, creating and rebuilding none (`index-missing` when there is none).
- Changed `--floating-zone` of `pimdir index` and of the one-shot answers to default to the zone the index records, `UTC` for a new index.
- Changed the `pimdir` duration flags to read through jiff instead of humantime (RUSTSEC-2026-0542, unmaintained): `2s`, `12h`, `90d`, `2 weeks`, a day being 24 hours; months and years are refused.

### Fixed

- Fixed a `VTIMEZONE` rule with a huge `INTERVAL` aborting the calendar derivation (a panic, or a division by zero in release), on writes, re-derivations and a sync engine's pull.
- Fixed a recurrence rule with a huge `INTERVAL` panicking the search expansion, or in release cutting every occurrence.
- Fixed an HTML comment closed by three dashes or more swallowing the rest of the body from the search index.
- Fixed `pimdir show` creating index.db, or wiping an index another release built, and a one-shot answer resetting the floating zone a sync engine indexed in.
- Fixed `pimdir queue status`, `item restore`, `item erase`, `item list`, `item show`, `store info` and the confirmation prompts printing control and bidi characters a server put in a mailbox name or an answer.
- Fixed a second rekey, a delivered copy and a lone minted copy losing their body and pending state, and making another source of the collection remove the message from its server.
- Fixed a rekey being quadratic in the collection, in the engine and in the store's write (16,000 members: 5.3 s to 65 ms in the engine).
- Fixed a mail binding joined to a message another source holds deriving an `Update` every run and never pushing a flag, and a first pairing removing a flag from the server that had it, by the page the message sat on.
- Fixed a queued `move` into the item's own collection deleting the message from its server: it parks as `move-in-place`.
- Fixed `store backup` and `store restore` leaving an empty or truncated body under its name that a rerun kept, keeping a corrupt body over the backup's, and leaving a backup stopped part way with its database.
- Fixed a listing at the base revision replacing the summary and sort key of a locally edited contact or event with the server's.
- Fixed a store kept on a rollback journal failing to be indexed or searched.
- Fixed a reader listing or count reading a sync's commit half way.
- Fixed `pimdir item export --output` creating its file readable by others.

## [0.15.1] - 2026-11-16

### Changed

- Changed the jiff dependency to 1 and ical-rs to 0.7.1.

## [0.15.0] - 2026-11-16

### Changed

- **BREAKING**: Changed the rusqlite dependency to 0.43 (libsqlite3-sys 0.40).

  Two libsqlite3-sys versions cannot link into one build, so a dependent naming rusqlite itself moves it to 0.43 with this release.

## [0.14.1] - 2026-11-16

### Changed

- Changed the pimalaya-cli dependency of the `cli` feature to 0.6: a `pimdir` listing prints `Next page:` after its items, and a header's values sit two spaces after its longest label.

## [0.14.0] - 2026-11-15

### Added

- **BREAKING**: Added pimdir storage-draft-13, sync-draft-13 and search-draft-13, vendored at the commit their tags name (pimdir 1c6442e; draft-12, tagged the same day, was never released by this crate).

  A store of an earlier draft is refused by every profile (`store-version`), and one whose sync state an earlier engine wrote too (`sync-version`), never migrated; an index of an earlier draft is rebuilt. The drafts carry the two fixes below. `sql::FORMAT_VERSION`, `sync::SYNC_VERSION` and `sql::search::FORMAT_VERSION` name them (`sql::search::FORMAT_DRAFT` is 13), and `init_store_meta` and `init_index_meta` changed their text.

- **BREAKING**: Added the refusal of a push for good (pimdir SYNC §4, §5, STORAGE §15.4): `PimdirPushOutcome::Refused { reason }`, the enum now `#[non_exhaustive]` and no longer `Copy`; `PimdirWriteOp::RefusePlacement { collection, handle, key, reason }` and `PimdirWriteOp::ClearRefusal { collection, handle }`; `PimdirLoaded::refused` with `load::PimdirRefusal`; `PimdirSyncEvent::Refused { handle, reason }` and `PimdirSyncReport::refused`.

  A connector answering `Refused` has the refusal recorded on the binding, its reason scrubbed; the engine then holds the change while it derives it under the same key, reporting it every run, pushes one derived under another key, and clears the refusal on the binding's next accepted push. The bindings gain `refusal`, `refused_key` and `refused_at`; `sql::REFUSE_BINDING`, `sql::CLEAR_REFUSAL`, `sql::LIST_REFUSALS` and `sql::LOAD_RECEIPT_REFUSALS` are new, `sql::PRUNE_RECEIPTS` keeps a receipt while a refusal stands on its item, and the sync vectors move to format 7 with cases 162 to 167.

- **BREAKING**: Added `PimdirActionStatus::Applied::refusals`, the refusals standing on the item an applied row acted on (`client::producer::PimdirReceiptRefusal`), with `PimdirReader::receipt_refusals` and `PimdirReader::refusals` (`client::producer::PimdirRefusalRow`). `pimdir queue status` prints a line per refusal and a `refusals` list in its JSON.

### Changed

- **BREAKING**: Changed every receipt to name the item its row acted on: a `set-flags`, an `update` or a `remove` the item it names when the store holds it, a `move` or a `copy` the item it left in its target, as an `add` named the item it created (pimdir STORAGE §15.2).

### Fixed

- Fixed the producer's gate refusing to withdraw a pending create: a `remove`, a `move` or a `copy` no longer asks a source binding the item as a pending create alone for its remove or its move, its server holding nothing yet (pimdir STORAGE §15.6). The vendored `load_item_capabilities` answers a fifth column, `held`.

  A message added to a Graph folder other than Drafts, rejected by every sync, could not be deleted while the source's deletes were off.

- Fixed a narrowed round dropping members its enumeration holds: the last page of a round a date filter narrowed finds absent only a member in scope its `enumerated` list lacks, whatever its date, and nothing when it hands none, counting what it kept in `PimdirSyncReport::unlisted` (pimdir SYNC §5).

  A band narrowed by arrival (Gmail's `before:`, Graph's `receivedDateTime`) dropped messages dated long before they arrived, which the server still held. The vendored `list_unstamped_bindings` changed its text, vectors 47 and 107 changed, 160 and 161 are new.

## [0.13.0] - 2026-11-15

### Changed

- **BREAKING**: Changed the `pimdir` binary to exit 3, not 2, when a command printed its result short of full success, following pimalaya-cli 0.5 (2 is a refused command line).
- Changed the mime-rs dependency to 0.2, which reads a structural field an entity states twice from its first occurrence, as the summaries and the extraction already did.
- Changed the pimalaya-cli dependency of the `cli` feature to 0.5.

## [0.12.0] - 2026-11-13

### Added

- **BREAKING**: Added pimdir storage-draft-11, sync-draft-11 and search-draft-11, vendored at the commit their tags name (pimdir 83102f0).

  A store of an earlier draft is refused by every profile (`store-version`), and one whose sync state an earlier engine wrote too (`sync-version`), never migrated; an index of an earlier draft is rebuilt. The schema holds `mail_summary.date` to an instant in UTC at seconds precision (`2026-08-01T10:00:00Z`) or `NULL` by `CHECK`. One statement is new, `sql::UNCONFIRM_ACTION`, and `sql::PARK_ACTION` changed its text. The vectors gain sync cases 155 to 159, two summary cases, two payload cases and eight search cases; every case passes, tests/pending_vectors.txt naming none.

- **BREAKING**: Added `mark` to `search::coverage::PimdirSearchReads`, set by `has:attachment`, under which mail's `bodiless` items count against an answer (pimdir SEARCH §8.2), with `PimdirSearchReads::counts_bodiless`, whether a `bodiless` item of a kind counts against an answer (a term reads a body of the kind, or the kind is mail under `has:attachment`), and `PimdirSearchReads::text`, what a text term reads. `pimdir search --json` and `pimdir count --json` print it as `coverage.local.reads.mark`.
- Added `summary::mail::first_occurrences`: a message with the later occurrences of `Content-Type`, `Content-Disposition`, `Content-Transfer-Encoding` and `Content-ID` cut out of the header block of each entity the walk of its parts reads, borrowed when no entity states one twice. It is what a reader hands a parser reading such a field from its last occurrence (pimdir STORAGE Annex A.1, SEARCH §6).
- Added `summary::mail::usable_date`, a message's `date` when it is one the store takes, and `PimdirHeldObject::agrees`, the agreement of pimdir STORAGE §9 between a row holding a body and a message's summary.

### Changed

- **BREAKING**: Changed a `PimdirMutation::Move` meeting a conflict, its item's or its binding's, to be refused, staging nothing in either collection (`PimdirMutateError::Unresolved`, `conflict-unresolved`, its `resolves` `None`), where it tombstoned the item under its conflict (pimdir SYNC §7). The drain parks a queued `move` naming the diverging body (pimdir STORAGE §15.2, §15.3), and a reader's overlay folds nothing for it, in the collection it leaves and in the one it names (§15.4). A `Copy` is unchanged. The conflict is settled first, by an `Edit` or a `Remove` stating `resolves`.
- **BREAKING**: Changed a rekey whose listing states `capped` to park while any based binding of its source resolves to no member, whatever their number, unless the run carries `accept_absences` (pimdir SYNC §8): nothing is written and `PimdirRekeyReport::parked` carries the counts, where each binding the cap left out was dropped as gone, or re-staged as a create where it held an edit.
- **BREAKING**: Changed `PimdirSearchCoverage::is_complete`: a `bodiless` item counts against an answer only while a term of the query reads a body of its kind, as a `missing` and an `unparseable` placement do, and, for mail, while the query holds `has:attachment` (pimdir SEARCH §8.2). It counted against every query, so a mailbox listed and not fetched never answered complete; it now does to `is:unread`, `tag:` or a `date:` over mail. `pimdir search` prints the count under the same terms.
- **BREAKING**: Changed `PimdirQuery::conjuncts` to read through a group that is itself a conjunct of the whole query, at any depth (pimdir SEARCH §8.2): `(collection:a) x` and `collection:a x` have one scope and one coverage, where a term inside any group ruled no collection out. A negated group, and one beside an `or`, still rule none out.
- Changed `PimdirQuery::reads` to have `thread:` read a message's body, a message holding none naming its parent by its `In-Reply-To` alone (pimdir SEARCH §8.2, §9).
- Changed the query language to read an `@` as an address in the value of a role field (`from:`, `to:`, `cc:`, `bcc:`, `attendee:`, `organizer:`, `email:`) and of `with:` alone (pimdir SEARCH §8.3, §8.5): in `subject:`, `title:`, `summary:`, `body:`, `person:`, `attachment:`, `mimetype:` and `location:` it is a character, so `subject:ledger@exa*` is a prefix, where its wildcard was refused (`invalid-wildcard`).
- Changed the walk of a message's parts to read the last child of a multipart no closing delimiter ends as a part, ended by the end of its entity, its size without the line break that entity ends with (pimdir STORAGE Annex A.1): a message cut short derives the attachment mark, the invitation and the stand-ins the same message closed does, where its last part gave none. A message summarised before keeps its row until its body enters again, Annex A staying at revision 3.
- Changed the search extraction to read an entity's `Content-Type`, `Content-Disposition` and `Content-Transfer-Encoding` from their first occurrence, as the summary does (pimdir SEARCH §6), where mime-rs read the last: a message stating one twice is parsed from `summary::mail::first_occurrences`.
- Changed a mail summary stating a `date` that is no instant in UTC at seconds precision (a fraction of a second, an offset, a date alone) to be stored with no date, as an empty one already was, where the text was stored and compared as written (pimdir STORAGE §13). `PimdirScope::contains` reads such a date as none, in every scope.
- Changed `PimdirMailSummary::sort_key` and the `alt:` key of `PimdirMailSummary::link_id` to read such a date as none, as the row stored does, where they read the text as stated (pimdir STORAGE Annex A.1).
- Changed a message's body entering under a `Message-ID` another row of the account holds another body under, a message it does not agree with on subject, date and sender, to record no stand-in and no `attachment` reference (pimdir STORAGE §14.3): a `part:` key stands for one message, where the second message's parts were recorded under the first's keys.
- Changed `PimdirStore::unconfirm_action` to run the canonical `unconfirm_action`, which alone writes the reserved prefix, and `fail_action` and the drain's parks to leave the `failed: ` of a text beginning with it to the canonical `park_action` (pimdir STORAGE §13, §15.2). What each records, counts, answers and refuses is unchanged.
- Changed `PimdirReader::search_coverage` to state what a text term reads, the store's `LIKE` reads searching a text, so an item holding no body still counts against them.

## [0.11.0] - 2026-11-13

### Added

- **BREAKING**: Added pimdir storage-draft-10, sync-draft-10 and search-draft-10, vendored at the commit their tags name (pimdir ae8b231), the four batches of those drafts: the schema and the statements, the payloads and identity, the text rules, then search.

  A store of an earlier draft is refused by every profile (`store-version`), and one whose sync state an earlier engine wrote too (`sync-version`), never migrated; an index of an earlier draft, or of an earlier text of this one (`index_meta.extraction` and `extractor`, the index `object_not_ok`), is rebuilt. The schema holds the shapes of STORAGE §13 by `CHECK` (a base stated without `base_present`, an empty handle, source, link id, collection id or account, a `seq` below 1, a negative size, a stamp or a scope bound in another shape), so a row in another shape is refused where it was stored. A consumer running the `sql` constants over its own binding follows the renamed parameters (`:before` in `purge_retained_before`, `release_before` and `release_bases_before`; `:hash` in `held_elsewhere`, `held_mail_summary` and `origin_for_link`; `:message_id` in `insert_in_reply_to`), the column `mail_in_reply_to.message_id`, the index `items_deleted`, and may bind `NULL` as the first cursor of every paged read. spec/queries/signatures.json, the specification's listing of every statement's parameters and columns, is vendored with them.

- **BREAKING**: Added `base` to `PimdirAction::SetFlags` and `PimdirMutation::SetFlags`, the flag set a producer read and changed into `flags` (pimdir STORAGE §15.3, SYNC §7).

  With it the owner applies the change alone to the set the item holds when the row is applied, so a flag a sync landed or removed since survives; over an unknown set the mutation is refused (`PimdirMutateError::UnknownFlags`, `flags-unknown`) and the row parks. `None` replaces the set as before. `PimdirFlags::changed` computes it. The search tag plan states the set it read.

- **BREAKING**: Added `if_match` and `resolves` to `PimdirAction::Remove` and to `PimdirMutation::Remove`, now a struct variant carrying its `handle`, on an update's rules (pimdir STORAGE §15.3, SYNC §7): a remove is refused, and its row parked, over another body than the one it names, over a conflict its `resolves` does not name, and over none when it states `resolves`.
- Added `codec::payload_outcome` and `PimdirPayloadOutcome`: a payload is valid, malformed or unknown (pimdir STORAGE §15.3).
- Added `PimdirMutation::Abandon { handle }` and `PimdirSyncEvent::Abandoned(handle)`: a delivered create whose member never arrived is given up on a person's request (pimdir SYNC §5, §7). Its binding is dropped and the item retained with its body and the flags staged on it; nothing is pushed. Any other placement refuses it (`PimdirMutateError::NotDelivered`, `create-not-delivered`).
- Added `PimdirStore::retry_action`, which puts a parked queue row back among the pending ones on request, its id and attempts kept (the canonical `retry_action`, pimdir STORAGE §15.5), and `pimdir queue retry`.
- Added `PimdirStore::unconfirm_action`, `PimdirActionStatus::Unconfirmed`, `PimdirParkedAction::unconfirmed`, `codec::UNCONFIRMED` and `codec::unconfirmed`: an intent whose outcome is unknown is parked under the reserved prefix `unconfirmed: `, which the crate writes and the status is read from (pimdir STORAGE §13, §15.2). No drain retries it; it leaves by `acknowledge_action`, `retry_action` or `drop_action`. An action the owner applies itself is refused (`PimdirError::NotIntent`, `action-not-intent`). `pimdir queue status` prints `unconfirmed`, and `pimdir queue list --parked` an `unconfirmed` member.
- Added `PimdirProducer::enqueue_batch` and `PimdirEnqueue`: several actions appended in one transaction, all or none, each gated once, a body in hand written only once the whole batch passed (pimdir STORAGE §15.1).
- Added `PimdirSourceStore::RUN`: a drain applies up to 64 pending actions in one transaction, a savepoint per action (pimdir STORAGE §15.2).
- Added `PimdirStore::local_dir`: the store's local/, the owner's own state, created `0700` on demand (pimdir STORAGE §3). An owner's open refuses one another user can reach (`store-exposed`).
- Added `PimdirLock::verify` and `PimdirError::LockReplaced` (`lock-replaced`): an owner checks before every write transaction that the lock file it holds is still the one at its path, and a producer before its enqueue, failing before anything is written (pimdir STORAGE §8).
- Added `PimdirReader::backup`, `PimdirStore::restore` and `PimdirReader::check_deep` (module `client::backup`, with `PimdirCopyReport`, `PimdirRestoreReport`, `PimdirDeepReport`, `PimdirForeignKeyViolation`), and `pimdir store backup`, `pimdir store restore` and `pimdir check --deep` (pimdir STORAGE §7, §8). A backup into a directory holding anything is refused (`PimdirError::Occupied`, `backup-occupied`). A restore answers a new incarnation.
- Added `format`, `user_version` and `hash_algo` to `version::PimdirStoreVersion`, `hash_algos` to `PimdirProgram`, and `Foreign`, `Mismatch` and `HashAlgo` to `PimdirVersionRefusal`: a release judges a store by the six columns of `store_meta` (pimdir STORAGE §4.2).
- Added `PimdirSyncEvent::Disagreed { handle, link_id }`, reported after the `Added` of a member minted apart from a pending or a delivered create of its source whose summary disagrees with its own (pimdir SYNC §5, §6).
- Added `PimdirLoaded::offered` and `PimdirHub::offered`: the `Created` placements a store offers a source that binds not their item, which a sync tells from the source's own creates. A storage servicing the seam itself fills it, or reports a disagreement with an offered copy too.
- Added `PimdirHeldObject`, a body a row of the account holds under a link id with the `subject`, `date` and `sender` its mail summary states.
- Added `PimdirActionHandle` and `PimdirStoreIdentity`: the handle an enqueue answers, the row's id under the store's id and incarnation, printed as the three joined by periods and parsed back; a bare id is a handle stating no store (pimdir STORAGE §4.5, §15.4).
- Added `PimdirActionStatus::Reset` and `PimdirError::Reset` (`store-reset`): the store is no longer the one a handle was answered in. `pimdir queue status` prints `reset`, and `pimdir queue cancel` refuses.
- Added `PimdirWaiting` and `PimdirWaitReason` (`OutOfScope`, `Delivered`, `Undeliverable`).
- Added `handle` beside `row` in the JSON of `pimdir item restore` and `pimdir tag`.
- Added `PimdirError::ScopeBound` (`scope-invalid`) and `PimdirScope::malformed_bound`: a scope bound that is not an instant in UTC at seconds precision (`2026-10-01T00:00:00Z`) is refused by `sync`, `prepare_sync` and `write` before anything is listed or written, where it was stored and compared as text.
- Added `sql::search::HIT` as the read of a hit's placements: the canonical statement, re-signed `(:seq, :account)` and answering every live placement of the hit with its kind and title.
- Added `search::extract::EXTRACTION` and `EXTRACTOR`, the revision of pimdir SEARCH §6 the crate extracts under and its own revision of what §6 leaves to an indexer (`io-pimdir 1`), both recorded in `index_meta` (pimdir SEARCH §2).

  `PimdirIndex::open` and `refresh` rebuild an index recording another of either, so every body is extracted and every calendar item expanded again; `open_query` answers from one, cutting its snippets from its own chunks. A consumer running the refresh coroutine over its own index compares the two constants with what it recorded and rebuilds or extracts again itself. `sql::search::SET_EXTRACTION` is the canonical statement restating them, which the crate, rebuilding, does not run.

- **BREAKING**: Added `search::coverage::PimdirSearchUnread`, the four counts pimdir SEARCH §8.2's `coverage` answers per kind (`bodiless`, `missing`, `unparseable`, `encrypted`), as `PimdirSearchLocal::kinds`, in place of `PimdirSearchLocal::bodiless`; `PimdirSearchReads` and `PimdirSearchLocal::reads`, the bodies a query's terms read; and `PimdirSearchCoverage::zone_differs`, set when a query states another zone than the index read floating times in and a date term can reach a calendar item.
- Added `PimdirQuery::conjuncts`, the terms standing as conjuncts of the whole query, which alone rule a collection out of the query's (pimdir SEARCH §8.2), and `PimdirQuery::reads`.
- Added `search::query::RELATIVE_DIGITS_MAX` (6) and `STAMP_DIGITS_MAX` (18).

- **BREAKING**: Added `PimdirPushResult::relocated`, which a connector sets on an accepted `Remove` carrying `to` that relocated the member rather than deleted it (SYNC §4): only a relocation delivers the destination's create.

- **BREAKING**: Added `PimdirWriteOp::DeliverPlacement`, the write recording a create delivered before any listing named it: an add accepted with no handle, or a relocation into it accepted (pimdir SYNC §2).

  A delivered create derives nothing, is never found absent and lands on the page naming its member, so a delete or a move staged on it is held, then pushed against the listed handle, rather than withdrawn. `PimdirSyncReport::waiting` names the delivered creates a run holds. A storage applying the write ops matches the new variant (`deliver_binding`).

- Added `PimdirHandle::is_provisional`, `PimdirPlacement::delivered` and `PimdirLinkId::is_landable`.
- Added the `version` module: `PimdirStoreVersion::opens` judges, I/O-free, whether a program of a profile (`PimdirProfile`), a schema version and a draft or released build (`PimdirFormat`, `PimdirProgram`) opens a store stating a `format_version`, a `version`, a `read_version` and a `produce_version`, answering a `PimdirVersionRefusal` otherwise (pimdir STORAGE §4.2).

  The draft rule every open runs goes through it and refuses the same stores as before. The released rule (an owner opens a version it knows, a reader a store whose `read_version` is at most its own, a producer one whose `produce_version` is too, and a release refuses every draft store) is implemented and tested, and run by no open until a release is built.

- **BREAKING**: Added `PimdirGcReport::temporaries` and the `temporaries` member of `pimdir gc --json`: the temporary files the collector unlinked.
- Added `PRAGMA journal_size_limit`, 4 MiB, on every connection, so a WAL one large write grew is truncated once checkpointed.
- Added a bounded `PRAGMA incremental_vacuum` after `purge_retained_before`, `evict_before`, `release_before` and `collect_garbage` when they removed rows: at most 4,096 free pages go back to the filesystem per call, where pimdir.db never shrank before.

### Changed

- **BREAKING**: Changed a tombstone to settle no conflict by itself (pimdir STORAGE §14, SYNC §7, §9): the write clears an item's conflict under a tombstone only when a `PimdirWriteOp::Settle` precedes its upsert, which a `Remove` resolving it states. A `Move` over a conflicted item, or any other tombstone, leaves the conflict standing with its diverging body kept, where every tombstone cleared it and released the body. A storage applying the write ops follows.
- **BREAKING**: Changed `codec::action_from_payload` to keep a payload unknown to the crate verbatim as `PimdirAction::Unknown`, read no further: a kind it holds no schema for, or a known one at another integer `v` than 1. It answered `PimdirActionError::UnknownVersion`. The drain leaves such a row pending and untouched, and `pending_actions` lists it, where a row at a later version was left out of every read. A known action stating no `v`, or one that is no integer, is malformed (`PimdirActionError::UnknownVersion(None)`) and parked, as before.
- **BREAKING**: Changed `PimdirArg::LookupObject` and `PimdirSourceStore::lookup_objects` to answer every row of the account holding a body under a link id (`Vec<PimdirHeldObject>`), where they answered one `PimdirObject`. An upgrade links a message's body only from a row agreeing with its summary on subject, date and sender, and on its size where it states one (pimdir SYNC §6).
- **BREAKING**: Changed a message to share its public id across mailboxes only with a row agreeing with its summary, and a bodiless row to be purged as held elsewhere only for a holder that agrees with it (pimdir STORAGE §9, §9.1, §11.1). A consumer running `sql::SEQ_FOR_LINK_ANY` over its own binding binds `:subject`, `:date` and `:sender`, `NULL` for another kind.
- **BREAKING**: Changed every enqueue of `PimdirProducer` to answer `PimdirEnqueued`, the row's `PimdirActionHandle` and the partial supports its gate computed, where it answered the row's id, `PimdirEnqueued::id` to `PimdirEnqueued::handle`, and `PimdirSearchTagAction::action` to the handle. A caller no longer runs `check` before it enqueues.
- **BREAKING**: Changed a release's judgement in `PimdirStoreVersion::opens` to refuse a marker that is not `storage-v` followed by the store's `version`. A draft's judgement is unchanged.
- Changed the drain to apply the queue in runs, one transaction each. A failed action is rolled back to its savepoint and parked or retried in the run's transaction; a run that fails to commit applies none of its actions.
- Changed `PimdirSearchTagPlan::stage` to enqueue its actions as one batch.
- Changed `fail_action` and the drain's parks to record a failure text beginning with `unconfirmed: ` behind `failed: `, so no failure reads as an unknown outcome.
- Changed `evict_before` to keep an item a delivered create binds (the canonical statement's new text, pimdir STORAGE §11.3).
- Changed a write to run `ensure_collection` once per collection of a batch and to read the link of an upserted handle once.
- Changed an enqueue naming a body by its hash alone to flush the body's shard directory before its row commits (pimdir STORAGE §5). `action_status`, `drop_action` and `cancel_action` take a handle or a bare id.
- **BREAKING**: Changed `PimdirSyncReport::waiting` from a count to the list of the creates left waiting, each with its reason; a create holding neither a body nor an origin is listed as undeliverable, where nothing counted it.
- **BREAKING**: Changed the search extraction to read `Subject`, the address fields, `In-Reply-To` and `References` from their first occurrence, as the summary does (pimdir SEARCH §6), where it read the last; and to read a text part disposed with any type but `inline` as an attachment. A body indexed before keeps its text until it is indexed again.
- Changed a card, an event, a task or a journal whose `UID` is empty, or of spaces and tabs alone, to be keyed by the `hash:` fallback, where the unusable `UID` was the key (pimdir STORAGE §9). An item named before keeps its key.
- Changed `pimdir queue status` and `pimdir queue cancel` to take a handle or an id, and `pimdir queue list` to print a `set-flags`'s base and a `remove`'s `if-match` and `resolves`.
- **BREAKING**: Changed `PimdirStore::collect_before` to `PimdirStore::evict_before`, its report `PimdirCollectReport` to `PimdirEvictReport` and `sql::COLLECT_BEFORE` to `sql::EVICT_BEFORE`, as the standard names the act an eviction (pimdir STORAGE §11.3). No alias is kept.
- **BREAKING**: Changed `PimdirActionStatus::Applied::collection` to an `Option<String>`: a receipt outlives the collection its row was queued on and then names none (pimdir STORAGE §15.4), where deleting the collection dropped the receipt and the action read `Unknown`. `pimdir queue status` prints such an action as applied on a collection deleted since, its JSON `collection` `null`.
- **BREAKING**: Changed `PimdirStore::raise_incarnation` to answer an incarnation a random stride (1 to 2^31) above the copy's and the replaced store's, no longer the next integer: two restores of one backup answer two incarnations (pimdir STORAGE §8).
- Changed a collection's delete to count in `PimdirChangeCursor::purges`, an empty collection's included, so a consumer of the change feed reconciles its keys (pimdir STORAGE §4.5); the search index does.
- Changed the index to expand again every live journal when its horizon rolls, a journal's summary not saying whether it recurs (pimdir SEARCH §7): a recurring journal kept the occurrences of the horizon it was indexed under.
- Changed a binding to be read as based from `base_present` alone, the one witness of a base (pimdir STORAGE §13).
- Changed the upgrade to fetch no provisional handle: a pending or a delivered create waits for the page that lands it.
- **BREAKING**: Changed the collector to unlink the temporary files a killed writer left: every period-prefixed regular file of objects/ and its shard directories, which `PimdirGcReport::strays` reported and left before.

  Under the collector's locks no writer holds one. A producer inside its staging window still fails the collection (`store-staging`), and so does a blob stream the owner still holds open (`store-in-flight`), which a collection ran beside before. `pimdir check` no longer fails for ever on such a file: `pimdir gc` removes it.

- Changed a producer's enqueue naming a body it did not stage to take the staging lock and verify the body first: one the blob directory does not hold (`NotFound`) or failing its name (`InvalidData`) is refused as `PimdirError::Io`, where it was queued, its object row recreated, before.
- Changed `PimdirStore::put_item` to stage its body before its transaction, under the process's writer lock, as `PimdirSourceStore::write` does.
- Changed `PimdirLock::running` and an owner's `PimdirBlobStage::writer` to wait out a collection of the same process.
- Changed the HTML reduction of the search index: a tag separates the text on its two sides unless it is an inline element's (`a`, `b`, `em`, `span`, `sub`, `sup` and the like), so the cells of a table are words apart. A body indexed before keeps its text until it is indexed again.
- **BREAKING**: Changed the CLI's lists to print as plain text, with no border, no colour and no truncation.

  A wide list prints one record per item, the way `git log` prints commits: a title line with its state words in parentheses, aligned `Label: value` fields, then its free text indented by four spaces, one blank line between items. `collection list` titles each collection by its id, `item list` each item by its seq with its flags as state words and its title as the text, `queue list` each action by its id, `search` and `thread` each hit by its seq with its snippet as the text, `search --threads` each thread by its id with one line per hit as the text, and `tag` each hit by its seq with one field per placement. The collection counts of `store info` print as columns two spaces apart under an uppercase header. The `--json` output is unchanged.

- **BREAKING**: Changed the CLI's records to print as aligned `Label: value` lines: `store info`, its collection counts following as a list, `item show`, each placement as the item's fields, its summary row's and each binding's, one blank line apart, `show`, then its body and note, and the thread of `thread`, then its hits.
- Changed the CLI's `--help` to print each paragraph on one line, no longer wrapped to the terminal width.
- Bumped pimalaya-cli to 0.4.

- **BREAKING**: Changed `PimdirSearchCoverage::is_complete` to pimdir SEARCH §8.2's complete answer, the lag included: a consumer no longer adds `local.behind == 0` itself.

  An answer is complete when the index lags the store by nothing, the temporal axis is full, no range left the horizon, no item was cut, no server answer was left partial, no kind counts a bodiless item, no kind counts a `missing` or an `unparseable` placement while a term of the query reads a body of that kind, and no kind counts an `encrypted` one while a text term is matched in the body or the attachment field (free text, `body:`, `attachment:`, `mimetype:`). A mailbox holding encrypted mail thus answers complete to a query of structured terms or of header fields, where a bodiless item still counts against every query. An answer behind the store read complete before; `zone_differs` leaves it complete.

- **BREAKING**: Changed the coverage in the CLI's `--json` output of `search`, `count` and `tag` (and their JSON Schemas): `coverage.local.bodiless` is `coverage.local.kinds`, per media type the four counts, `coverage.local.reads` names what the query's terms read (`kinds`, `content`), `coverage.zoneDiffers` is new, and `coverage.complete` is false while the index is behind the store. The text output prints the counts the query's terms read, and the differing zone under a complete coverage too.
- **BREAKING**: Changed `PimdirQuery` to keep a parenthesised group as `PimdirQuery::Group`, matching as its operand does: a term inside a group, like one negated or beside an `or`, rules no collection out of the query's, so `(collection:a) x` is covered over every collection where `collection:a x` is over one (pimdir SEARCH §8.2). A consumer matching on the AST adds the arm.
- **BREAKING**: Changed the query grammar to refuse a relative amount of more than six digits (`invalid-date`) and a change stamp of more than eighteen (`invalid-value`), leading zeros counted (pimdir SEARCH §8.3).
- Changed a date range to be open on a side no instant of a four-digit year states (pimdir SEARCH §8.1): an end past `9999-12-31T23:59:59Z`, so `date:2026..9999` answers what `date:2026..` does where it answered nothing, and a start before `0000-01-01T00:00:00Z`, which `date:999999d` reaches, where it was clamped to year 0.
- Changed U+0000 in a text term to separate tokens as a space does (pimdir SEARCH §8.5), where the query failed as unanswerable.

### Removed

- **BREAKING**: Removed `PimdirProducer::enqueue_with_body_checked`: `enqueue_with_body` answers what it did.
- **BREAKING**: Removed the `charset-tables` feature: encoding_rs is a plain dependency, `no_std` with `alloc`, so every build decodes every charset label of the WHATWG Encoding Standard (pimdir STORAGE Annex A.1). A build without the feature kept a word in a legacy charset as its literal text and derived other subjects and `alt:` keys silently. Drop the feature from a dependency line naming it; no alias is kept.
- **BREAKING**: Removed `sql::COMPONENT_OF` and `sql::search::MATCH_OBJECTS_RANKED`, cut by the standard and run by nothing here, and `sql::search::LOAD_HIT_PLACEMENTS`, the crate's own text of what `sql::search::HIT` now reads.
- Removed the comfy-table dependency.

### Fixed

- Fixed a mail or file summary stating a size above `i64::MAX` being stored as a negative size: it is written with an unknown size.
- Fixed a collection running beside a verb or a write it did not see: the in-flight guard of `collect_garbage`, `evict_before` and `release_before` was read before their locks were taken, so a verb starting in between had the body it streamed swept as an orphan.
- Fixed a blob write keeping a name another writer made without flushing its shard directory, a batch flushing one shard directory once per body, and the collector, so an erase, answering before its unlinks were durable.
- Fixed the projection reading a known flag set as differing from an unknown base, or an unknown set from a known base: flag sets differ only when both are known (pimdir SYNC §3).
- Fixed a calendar item leaving every agenda when the index re-expanded it (a horizon roll, another time zone database or floating zone) while its body could not be read: it keeps its occurrences, is recorded missing, and is expanded again by the pass that reads its body.
- Fixed the HTML reduction joining the last word before a tag with the first after it when a line break preceded them.
- Fixed the HTML reduction indexing the content of a `head`, `style`, `script` or `template` element whose tag carries attributes: `<style type="text/css">` put the CSS of a styled message in the index. Each is told by its tag's first word (pimdir SEARCH §6); `EXTRACTOR` being recorded from this change on, every index is built again under it.
- Fixed control and bidi characters from the store (collection ids and names, titles, flags, link ids, sort keys, summary values, handles, queue payloads and errors) reaching the terminal through the text output of `collection list`, `item list`, `item show`, `queue list` and `store info`.
- Fixed control and bidi characters from the store (titles, snippets, extracted texts, kinds, collections, link ids, accounts, thread ids, the coverage's media types and dates, and `check`'s hashes, paths, rows and collections) reaching the terminal through the text output of `search`, `thread`, `tag`, `show` and `check`: a crafted subject could write the clipboard through OSC 52 or clear the screen.

## [0.10.0] - 2026-11-11

### Added

- **BREAKING**: Added pimdir storage-draft-09, sync-draft-08 and search-draft-09, vendored at pimdir 8c16bb1.

  A placement is keyed by an integer id every child row carries, and its system flags (`\Seen`, `\Answered`, `\Flagged`, `\Draft`, `\Deleted`) are bits of the row (`codec::MARKS`, `codec::mark_of`, `codec::marks_to_int`), every other flag a row. A store an earlier draft wrote is refused by every profile, never migrated: move it aside and let its sync engine create a new one.

- **BREAKING**: Added `PimdirLoadSelection::Owing`, what a delta loads: the handles it lists and drops, and every placement that may owe the source a push (`list_owing_bindings`, `list_lacking_items`), then its hints' `Links`. A delta no longer loads the whole collection.

- **BREAKING**: Added `PimdirSearchClock::zone`, the IANA zone a query's years, months, days, `today` and `yesterday` are read in, UTC when `None`.

  `PimdirSearchClock` and `PimdirSearchOptions` are no longer `Copy`. An unknown zone fails the query with `index-zone-unknown`.

- Added `PimdirIndex::floating_zone`, the zone the index reads floating times in, on a query-only handle too.
- Added `--zone` to the CLI's `search`, `count` and `tag`, the index's floating zone by default.

### Changed

- Prepared every statement the client runs more than once through a statement cache of 256 per connection, enabling rusqlite's `cache` feature (hashlink, and foldhash under Zlib, now allowed): a store write prepared its trigger programs on every call. Writing 50,000 listed messages through the owner went from 17.6 s to 2.9 s.

## [0.9.0] - 2026-11-04

### Added

- **BREAKING**: Added pimdir storage-draft-08, sync-draft-07 and search-draft-08, vendored at pimdir 6687cf4.

  A store an earlier draft or release wrote is refused by every profile, never migrated. Move it aside and let its sync engine create a new one.

- **BREAKING**: Added `PimdirRemoteSnapshot::capped`, `narrowed` and `enumerated`.

  A capped round finds no member absent, its unlisted bindings counted in `PimdirSyncReport::unlisted`. A round a date filter narrowed finds absent only a member dated before it opened or missing from the enumeration.

- **BREAKING**: Added `PimdirLoadSelection::LastPage`, the load of a round's last page answering `PimdirLoaded::unstamped`.
- **BREAKING**: Added `PimdirMutate::syncs_target`.

  A `Copy` or `Move` of a bodiless placement fails `Undeliverable` unless its source syncs the target. The default refuses.

- **BREAKING**: Added `PimdirYield::WantsVerify`, `PimdirArg::Verified` and `PimdirSourceStore::verify`.

  A sync and a `Full` upgrade check every body against its name before pushing or keeping it, a failing one counted in `PimdirSyncReport::unreadable`.

- **BREAKING**: Added `PimdirAction::Add::supersedes` and `codec::PimdirSupersedes`.

  The drain applies the add and tombstones the superseded item in one transaction. The sync holds the tombstone's delete while the create is pending.

- **BREAKING**: Added `placement::PimdirSuperseded` and `PimdirChangeKind::Add::supersedes`, the member a superseding create may be moved from.
- **BREAKING**: Added `PimdirLoaded::conflicted`, `held` and `superseded`.
- **BREAKING**: Added `PimdirArgError::UnresumablePage` and `IncompleteListing`, refusing a page that breaks the seam.
- **BREAKING**: Added `coroutine::PimdirWritten`, which `PimdirSourceStore::write` now answers.
- **BREAKING**: Added `PimdirWriteOp::Overrule` and `PimdirWriteOp::Settle`.
- **BREAKING**: Added `PimdirDrainReport::overruled`; `PimdirDrainReport` is no longer `Copy`.
- **BREAKING**: Added `PimdirSyncOptions::accept_absences` and the `accept_absences` argument of `PimdirSourceStore::rekey`.

  A round or rekey finding more than half of at least 20 based bindings absent, or naming no member at all, writes no drop and reports `parked`.

- **BREAKING**: Added `PimdirError::Exposed` (`store-exposed`).

  A store whose directory or pimdir.db another user can read or write is refused before anything is opened: `chmod 700` its directory and `chmod 600` its pimdir.db.

- Added the `collection-rename` and `collection-delete` intents: `intent::PimdirCollectionRename`, `PimdirCollectionDelete`, `PimdirProducer::enqueue_collection_rename` and `enqueue_collection_delete`.
- Added `intent::PimdirSubmit`, the `submit` intent payload.
- Added `capability::COLLECTION_RENAME`, `COLLECTION_DELETE`, `MAIL_FLAGS_IMPORTANT`, `MAIL_FLAGS_JUNK` and `CALENDAR_COMPONENT_VTODO`.
- Added `capability::gating`, `capability::is_name`, `capability::is_vtodo` and `capability::check_vtodo`.
- Added `PimdirReader::accepts_vtodo`.
- Added the owner's capability backstop: the drain parks an intent whose source declares capabilities but not the one it needs.
- Added round heads: `PimdirRemote::heads`, `PimdirHeads`, `PimdirHead`, `PimdirListing::Head`, `PimdirEnumerate::head`, `PimdirRemoteSnapshot::band_head` and `PimdirSync::heads`.

  A run resuming an open round asks for its head first, so a message arriving during a long round shows at the next run.

- Added `PimdirSyncEvent::Overruled` and `mutate::PimdirMutateReport`.

  A body a conflict policy does not adopt is kept as a retained item under `dup:<link>#<handle>` and reported.

- Added `upgrade::PimdirUpgradeOptions` and `PimdirUpgrade::options`: an upgrade fetching a divergence's body settles it by the source's policy.
- Added `PimdirSyncReport::unanswered` and `rejected_creates`.
- Added `PimdirError::Version` (`store-version`), `Damaged` (`store-corrupt`), `SyncVersion` (`sync-version`) and `Foreign` (`store-foreign`).
- Added `PimdirError::Queued` (`collection-queued`), `Role` (`collection-role`), `Flag` (`flag-invalid`) and `Cutoff` (`cutoff-invalid`).
- Added `PimdirError::Synced` and `NotUnbound`, refusing an item kept on the device in the wrong collection.
- Added `PimdirError::code()` and `PimdirMutateError::code()`, a stable kebab-case code per failure.
- Added `client::PimdirSqlError` and `client::PimdirJsonError`.
- Added `sql::FORMAT_VERSION` and `sync::SYNC_VERSION`.
- Added `PimdirReader::store_meta` and `PimdirStoreMeta`.
- Added `PimdirStore::raise_incarnation`, run by a restore on its copy.
- Added `PimdirStore::open_current`, an owner open that creates no store.
- Added the re-derivation of a store written at Annex A revision 1 or 2 when an owner opens it.
- Added quarantine/: `PimdirStore::quarantine`, `PimdirStore::empty_quarantine`, `PimdirQuarantineReport` and `PimdirBlobs::quarantined`.

  A body failing its name, or a link where a body belongs, is moved there by the owner and never read nor collected.

- Added `PimdirBlobs::verified`, `PimdirBlobs::scan`, `PimdirBlobScan`, `PimdirBlobStray` and `PimdirGcReport::strays`.
- Added `PimdirHash::is_name` and `PimdirHash::LEN`.
- Added `client::blobs::PimdirBlobStage` and `PimdirStore::blob_stage`.
- Added `PimdirProducer::stage_body`, `blob_writer`, `enqueue_add`, `enqueue_with_body`, `enqueue_with_body_checked`, `enqueue_foreign_body` and `probe`.
- Added `PimdirEnqueued` and `client::PimdirDuplicate`.
- Added `PimdirStore::put_item`, `PimdirStore::ensure_time_collection` and `client::TIME_COLLECTION`, for cards and calendar items kept on the device.
- Added `PimdirStore::erase`, `PimdirEraseReport` and, with `search`, `PimdirIndex::erase`.

  The erasure removes the item's key, subject, addresses and body words from both database files and their WAL.

- Added `PimdirReader::replied`.
- Added `PimdirAddressCursor` and `PimdirAddressPlacement::cursor`.
- Added `PimdirLinkId::from_hint`, `PimdirLinkId::stated` and `PimdirLinkId::RESERVED`.
- Added `collection::role_of_kind`.
- Added `codec::action_from_payload` and `PimdirActionError::UnknownField`, reading payloads strictly against their schemas.
- Added `codec::is_flag` and `PimdirFlags::invalid`.
- Added `PimdirHub::conflicted`.
- Added `security::path_component` and `security::scrub`.
- Added `client::modes`.
- Added `summary::mail::message_id`, `display_name` and `MIME_ENTITIES_MAX`, and made `charset_decode` public.
- Added `summary::mail::MIME_DEPTH_MAX`.
- Added `summary::mail::attachments`, `PimdirAttachmentWalk`, `parts` and `PimdirParts`, the streamed walk of a message's parts.
- Added attachment stand-ins: a body entering the store records a file at `Meta` per attachment in the account's attachments collection.
- Added `summary::strip_instructions` and `summary::calendar::unschedule`.
- Added `PimdirTaskSummary::created`, `contacts`, `priority`, `categories`, `parent` and `mid`, with the `contact`, `parent` and `source` references.
- Added `search::coverage::PimdirSearchCoverage` and `PimdirReader::search_coverage`.
- Added the experimental `search` feature: the full-text index of pimdir SEARCH.md, `client::search::PimdirIndex`.

  It covers mail, contacts, calendar items with their occurrences, tasks and files, threads, relative dates and tagging. Its API and index may change in a minor release.

- Added `PimdirIndex::refresh`, `search`, `count`, `threads`, `thread`, `tag`, `item` and `occurrences_between`.
- Added `PimdirIndex::open_query`, the query client's read-only open.
- Added `search::refresh::PimdirIndexRefresh`, the index refresh as an I/O-free coroutine.
- Added `search::fold::text`, `search::fold::value` and `search::tag::flags`.
- Added the `charset-tables` feature, enabled by `client`, decoding every WHATWG charset label.
- Added the CLI verbs `index`, `search`, `count`, `show`, `thread` and `tag`.
- Added the CLI verbs `item erase` and `quarantine empty`.
- Added the `--floating-zone` option to the CLI's refreshing verbs.
- Added the store id, incarnation and versions to `pimdir store info`.
- Added the jiff, rustix (Unix, `client`), encoding_rs (`charset-tables`), mime-rs, vcard-rs, ical-rs, unicode-normalization (`search`) and simd-cpu (`cli`) dependencies.

### Changed

- **BREAKING**: Changed the `pimdir` JSON output to follow pimalaya-cli 0.3's contract.

  Every failure carries a `code`, partial outcomes exit 2, listings page with `nextPage` and `--page-token`, and `item export` prints `{data, size, mime}` under `--json`. Queue ids print as `row` and enum values in kebab-case.

- **BREAKING**: Changed `pimdir collection list --json` to print `{"collections": [...]}`.
- **BREAKING**: Changed `load::PimdirLoadScope` to `PimdirLoadSelection`, and `WantsLoad::scope` to `selection`.
- **BREAKING**: Changed `PimdirSourceStore::upgrade` to take `PimdirUpgradeOptions`.
- **BREAKING**: Changed `PimdirSourceStore::mutate` to answer `PimdirMutateReport`, and `PimdirUpgradeReport` to carry `events`.
- **BREAKING**: Changed `PimdirMutation::Edit` to carry `if_match`, `overrule` and `resolves`.

  An edit is refused once another write replaced its body, or when it meets a conflict it does not name in `resolves`.

- **BREAKING**: Changed `PimdirAction::Update` to carry `if_match` and `resolves`.
- **BREAKING**: Changed `PimdirChangeKind::SetFlags` to `SetFlags { handle, add, remove }`, flag deltas a connector applies.
- **BREAKING**: Changed `PimdirWriteOp::SetRoundCursor::cursor` to an `Option`.
- **BREAKING**: Changed `PimdirParked::guard` to take whether the listing named a member.
- **BREAKING**: Changed `PimdirChangeCursor` to carry the store's id and incarnation; it is no longer `Copy`.
- **BREAKING**: Changed `PimdirReader::address_placements` to answer a page.
- **BREAKING**: Changed `PimdirReader::count_mail_by_day`'s `shift` to minutes east of UTC.
- **BREAKING**: Changed `hash::PimdirHasher` to a BLAKE3 hasher of its own, and `PimdirBlobs::open` to take the store directory alone.
- **BREAKING**: Changed `PimdirBlobs` to be read-only, its `writer` gone.
- **BREAKING**: Changed `PimdirBlobs::reader` to answer a verifying `PimdirBlobReader`.
- **BREAKING**: Changed `PimdirBlobs::path` to answer `io::Result<PathBuf>`, refusing a malformed name.
- **BREAKING**: Changed `PimdirBlobWriter::commit` to take no hash and answer the `PimdirObject` written.
- **BREAKING**: Changed `PimdirGcReport` to no longer be `Copy`.
- **BREAKING**: Changed `PimdirError::Sql`, `Unanswerable` and `Json` to wrap `PimdirSqlError` and `PimdirJsonError`.
- **BREAKING**: Changed `PimdirError`, `PimdirSyncEvent`, `PimdirEnumerated` and `PimdirQueryErrorCode` to `#[non_exhaustive]`.
- **BREAKING**: Changed `intent::PimdirSubmit`, `PimdirCollectionCreate` and `PimdirInvitation` to require their `source`.
- **BREAKING**: Changed `intent::PimdirSubmit` to carry no `object`, the message being the row's body.
- **BREAKING**: Changed `PimdirInvitation::from_payload` to refuse a `partstat` outside `ACCEPTED`, `TENTATIVE` and `DECLINED`.
- **BREAKING**: Changed the queue codec to refuse malformed payloads, with `PimdirActionError::Invalid` for a member of another type.
- **BREAKING**: Changed the blob layout to one shard level, `objects/<hash[0:2]>/<hash>`, and `pimdir export` to dump format 3.
- **BREAKING**: Changed the summaries to pimdir Annex A revision 3.

  Message headers, dates and encoded words are read as Annex A.1 states. The `alt:` key of a message without a usable `Message-ID` moves with them.

- Bumped the minimum Rust version to 1.89.
- Bumped rusqlite to 0.41 and pimalaya-cli to 0.3.
- Changed `PimdirStore::replace_action` to refuse a change its schema refuses.
- Changed `PimdirStore::delete_collection` to refuse a collection holding queue rows.
- Changed `PimdirStore::set_collection_role` to refuse a role outside the collection's kind.
- Changed `PimdirStore::cancel_action` and the owner verbs of `pimdir` to open the store without creating it.
- Changed `PimdirBlobs::get` to answer `None` for a body failing its name.
- Changed `PimdirSourceStore::lookup_objects` and `verify` to answer only bodies checked whole.
- Changed blob writes to never replace a file, a held name being verified and kept.
- Changed every body read to follow no symbolic link under objects/.
- Changed the collector to unlink only regular files with a well-formed name in a well-formed shard.
- Changed `bindings.shared_object` to pin the body it names.
- Changed the producer to hold the staging lock only while it stages.
- Changed the journal mode to be set only when the owner creates the store.
- Changed every connection to `synchronous = FULL` and `secure_delete = ON`, plus `fullfsync = ON` on Apple platforms.
- Changed the store to free pages incrementally (`auto_vacuum = INCREMENTAL`).
- Changed the schema to close `collections.kind` and `items.level`.
- Changed flags, `In-Reply-To` ids and task categories to be stored as rows, no column holding JSON.
- Changed a public id to be shared within a kind alone.
- Changed the key of a stated identity beginning with a reserved prefix to `esc:` and the identity.
- Changed the attachment mark to pimdir's rule: a non-text part without `Content-Disposition` or `Content-ID` counts, protocol parts never do.
- Changed the walk of a message's parts to read at most 10,000 entities.
- Changed calendar sort keys to resolve a skipped wall time to the offset before the gap, and a `TZID` without `VTIMEZONE` through the IANA database.
- Changed the drain to skip, not park, a row stating an unknown payload version, and readers to skip it too.
- Changed the drain to park an `add` or `update` whose payload `object` is not the row's body.
- Changed the cross-source merge to move `shared_object` only with a settled body, and to record an unread divergence as `manual`.
- Changed `PreferLocal` to keep the member body it pushes over, and to push nothing over a member listed without a body.
- Changed a sync to report an event only once the write recording it committed.
- Changed a pull recorded as a divergence to report `Conflicted` rather than `ContentChanged`.
- Changed a rekey to drop every old handle before any upsert.
- Changed a reader, a producer and `open_current` to refuse a foreign database as `store-foreign`.
- Changed the package to leave out AGENTS.md, CLAUDE.md, deny.toml and the Nix files.
- Changed the `pimdir` binary to install the CPU token of the base64 and quoted-printable SIMD codecs.

### Removed

- **BREAKING**: Removed the reconciliation of earlier drafts' stores, with the `repair_*` and `backfill_*` statements.
- **BREAKING**: Removed `PimdirError::Stale`, replaced by `PimdirError::Version`.
- **BREAKING**: Removed `hash::PimdirHashAlgo`, `PimdirStore::open_with_hash` and every `hash_algo()` method.
- **BREAKING**: Removed `PimdirBlobs::files`, replaced by `PimdirBlobs::scan`.
- **BREAKING**: Removed `codec::flags_to_json`, `flags_from_json`, `ids_to_json` and `ids_from_json`.
- Removed the sha2 dependency.

### Fixed

- Fixed `pimdir json-schema` leaving out `queue status`.
- Fixed a stack overflow deriving a deeply nested message or calendar body.
- Fixed a boundary that another boundary begins with (`d1` and `d10`) closing the wrong multipart level.
- Fixed the invitation of a quoted-printable or base64 calendar part holding non-UTF-8 octets.
- Fixed a mail row keeping its listing's guess for the attachment mark and invitation once its body is held.
- Fixed a round page with `last` false and no cursor dropping every unlisted binding.
- Fixed a rekey answered by a delta dropping every placement it did not list.
- Fixed a delta page that is not the last landing its checkpoint.
- Fixed a page landing a pending create over a mutable member holding another body, or no body, without a conflict.
- Fixed the drain applying a queued `add` or `update` whose body the blob store lacks.
- Fixed a queued `update`, `set-flags` or `remove` on an unbound item never applying.
- Fixed a queued `update` or engine `Edit` made before a conflict settling it and discarding the diverging body.
- Fixed a person's `Edit` leaving the item's cross-source conflict standing.
- Fixed a local edit being dropped when another source's bodiless pull emptied the shared item.
- Fixed a `PreferRemote` bodiless pull letting the next run push over the server's unread body.
- Fixed a conflict policy discarding the body it did not adopt.
- Fixed a second source's message reusing a held `Message-ID` joining an item it does not agree with.
- Fixed an add accepted with no handle reporting `Created` for a member nothing named.
- Fixed a pending create or edit pushing a body that is missing or fails its name.
- Fixed a listing carrying the body a local edit holds being recorded as a conflict.
- Fixed a flag merge without a base dropping the local flags.
- Fixed a mutable handle stating a new identity dropping the local edit of the old one.
- Fixed a rekey re-staging an edit under an old handle the renumbering may have reused.
- Fixed a rekey matching two copies of one hint by key order rather than by body.
- Fixed an upgrade counting a `Full` fetch answering no body as fetched.
- Fixed two writers storing one body sharing their temporary file.
- Fixed the owner open dropping the tables of a database another program wrote.

### Security

- Created every directory of a store `0700` and every file `0600` on Unix.
- Opened every lock file without following a link.
- Opened every database with `trusted_schema = OFF` and `SQLITE_DBCONFIG_DEFENSIVE`.
- Scrubbed credentials from the error a parked queue row records.

## [0.8.0] - 2026-10-10

### Added

- Added `PimdirReader::sum_mail` and `PimdirMailSum`.

  They count the messages, known size and unknown sizes of a `[since, until)` range under the chips and a `held` chip, a message filed in several collections counted once (pimdir STORAGE §14.1).

- Added `PimdirStore::release_before` and `PimdirReleaseReport`.

  The bodies of a set of mail collections below a date go back to `Meta`, headers kept, and those still needed are kept (pimdir STORAGE §11.4).

- Added the `reference` module: `PimdirEndpoint`, `PimdirReference`, `PimdirReferenceRole` and `PimdirReferenceOrigin`.

  References between items follow pimdir STORAGE §14.2. The `item_reference` table, its index and its trigger are added on open to an older store, whose reader reads none.

- Added `PimdirStore::add_reference` and `PimdirStore::remove_reference`.

  `add_reference` answers the reference standing, `None` for an end missing or under a writer-derived key.

- Added `PimdirReader::references_from` and `PimdirReader::references_to`.
- Added automatic references, origin `auto`, written by any write recording a summary.

  The canonical rules link a mail's senders (`link_senders_of`) and invitation (`link_invitations_of`), a contact's mail (`link_mail_from`) and a calendar item's invitations (`link_invitations_to`), never touching a person's.

- Added files, the `application/octet-stream` kind (`summary::file::KIND`).

  The `file_summary` table and the trigger collecting unreferenced stand-ins are added on open to an older store (pimdir STORAGE §14.3, Annex A.7).

- Added `PimdirFileSummary`, `PimdirSummary::File` and file pages in `list_summaries`.
- Added `summary::file::part_key`, `None` for a message under a writer-derived key.
- Added `PimdirStore::put_file`, for a file holding no body in a collection no source syncs.
- Added `PimdirStore::delete_unbound`.
- Added `PimdirReader::list_attachments` and `PimdirAttachment`.
- Added `PimdirReader::list_attachments_by_account`, `PimdirAccountAttachment` and its cursor.
- Added `PimdirError::NotFiles`.
- Added the `attachments` role for a file collection.

  An older store's `collections` table is rebuilt on open to take it.

- Added `PimdirMailSummary::invitation`, the `UID` of the first `text/calendar` part.

  It is derived with the body and kept over a write without it (Annex A.1). Opening an older store adds the column and its index, and backfills it from the held bodies.

- Added `PimdirReader::get_mail_row`.
- Added `PimdirReader::describe_endpoint` and `PimdirEndpointView`.
- Added `PimdirReader::search_contacts`, `search_files`, and `search_calendar` with `PimdirCalendarHit`.
- Added `PimdirLinkId::is_derived`.

### Changed

- **BREAKING**: Changed `PimdirReader::count_mail`, `count_mail_by_day`, `count_unread` and `list_mail_page_filtered` to take a `since` floor on the sort key.

  The floor is seeked on the index, and undated mail falls below it. `None` keeps the previous behaviour (pimdir STORAGE §14.1).

- Changed the reconciliation on open to compare every table, index and trigger with the canonical schema's own text.

  A table that differs is rebuilt and an index or trigger that does is recreated, comments and whitespace aside. A store missing a core table is refused before anything is reconciled (pimdir STORAGE §6).

- Changed a listing over a file holding its body to keep the size the body gave (Annex A.7).
- Changed a listing or a `Meta` fetch over a mail placement holding its body to keep the size read from the body (Annex A.1).

  As for the attachment mark, a source's stated size no longer overrides it.

- Changed `PimdirStore::delete_collection` to skip the store-wide refcount recompute when the collection held no object pointer.
- Changed `PimdirStore::collect_before` to keep a mail a person's reference names (pimdir STORAGE §11.3).

## [0.7.0] - 2026-10-09

### Added

- Added the `collection-create` intent (pimdir Annex B.2).

  It comes with `capability::COLLECTION_CREATE`, declared in every domain's list, `intent::PimdirCollectionCreate`, and `PimdirProducer::enqueue_collection_create`, which anchors it on its parent and names its performer.

- Added an optional `recurrence_id` to the `calendar-reply` and `calendar-cancel` intents, replying to or cancelling one occurrence.

  It is gated on `capability::CALENDAR_REPLY_OCCURRENCE` and `CALENDAR_CANCEL_OCCURRENCE`. `PimdirInvitation`, `PimdirIntentItem` and `PimdirPartstat` build and read both payloads.

- Added queue receipts (STORAGE §15.4).

  The drain records what each applied row became, the `seq` an `add` created included, and prunes them after seven days. The `receipts` table is created on open in an older store.

- Added `PimdirProducer::action_status` and `PimdirReader::action_status`, following a row by the id its enqueue answered.
- Added `pimdir queue status`.
- Added `PimdirActionError::Invalid`, a payload field breaking its intent's rule.
- Added `PimdirStore::acknowledge_action(id, seq)`.

  It removes an intent its owner performed, releasing its pin and recording its receipt, so `action_status` answers `Applied` rather than `Unknown`. `drop_action` stays the withdrawal, without a receipt.

- Added `PimdirStore::set_collection_role` and `PimdirCollection::role` (pimdir STORAGE §14).

  `collections.role`, its index and its triggers are added on open to a store written by 0.6. A reader of one not reconciled yet reads `None`.

- Added the `ROLE` column of `pimdir collection list`.
- Added `PimdirSyncOptions::scope` and `PimdirScope`, a `[since, until)` range on the summary `date` (pimdir SYNC §4, §5).

  Mail only, else `PimdirError::Scope`. Deletes are inferred at the last page in scope only, and out-of-scope creates wait, counted in `PimdirSyncReport::waiting`.

- Added paged sync rounds, landed page by page and resumed from their cursor.

  They write through `PimdirWriteOp::OpenRound`, `Stamp`, `SetRoundCursor`, `CloseRound` and `SetCoverage`, and restart on `PimdirEnumerated::CursorRejected`.

- Added band rounds: `band` on `PimdirWriteOp::OpenRound` and `PimdirRound`, recorded in `sources.round_band` (pimdir SYNC §5).

  A band round infers no delete of an undated member. An open round resumes only as the kind of round it opened as. `sources.round_band` is added on open to an older store.

- Added `PimdirRemote::scope_bound` and `PimdirSync::scope_bound`.

  A connector bound to no scope widens a round by the band alone.

- Added `PimdirSync::report` and `PimdirSourceStore::prepare_sync`, for a runner of its own.
- Added coverage: `PimdirCoverage`, `PimdirRound` and `PimdirCursor`.

  The columns, `bindings.round` and their trigger are added on open to an older store.

- Added `PimdirLoaded::coverage`, `round` and `unstamped`.
- Added `PimdirCollection::coverage` and `PimdirReader::list_coverage`.
- Added the `COVERED` column of `pimdir collection list`.
- Added the mail reads: `PimdirReader::count_mail`, `count_mail_by_day`, `count_unread`, `list_mail_page_filtered` and `search_mail`.

  They come with `PimdirMailFilter`, `PimdirMailCursor`, `PimdirMailEntry`, `PimdirDayCount` and `reader::like_pattern`. The pages walk the `items_by_sort_global` index across collections, created on open in an older store (pimdir STORAGE §9.3).

- Added `PimdirStore::collect_before`, the owner's manual collection of a mail collection below a date (STORAGE §11.3).
- Added `summary::mail::derive_meta` and `summary::mail::meta_attachment`, reading the attachment mark without the body (Annex A.1).

  A body-less listing or fetch never writes the mark over a walked one.

### Changed

- **BREAKING**: Changed `PimdirRemote::enumerate` to take a `PimdirEnumerate` and answer a `PimdirEnumerated`.

  `PimdirEnumerate` carries the `PimdirListing` (delta, or round from a cursor) and the scope. `PimdirYield::WantsEnumerate` and `PimdirArg::Enumerate` follow.

- **BREAKING**: Changed `PimdirRemoteItem` to carry its `PimdirRemoteMeta`: hint, summary, sort key and optional body.
- **BREAKING**: Changed `PimdirRemoteSnapshot` to a page, with `last`, `cursor`, an optional `checkpoint` and the constructors `round`, `page` and `delta`.
- **BREAKING**: Changed `PimdirSyncOptions` to `Clone` without `Copy`.
- **BREAKING**: Renamed `PimdirMutateError::Probed` to `Unnamed`.
- Changed the sync to name every listed member in the page that lists it, landing a pending create its hint holds.
- Changed a pull to take the member's meta and carried body, the level lowered to `Meta`.
- Changed the rekey to list every page before its one batch and name members from their meta, fetching nothing.
- Changed a write to refuse an unnamed upsert of an unbound handle with `PimdirError::Unnamed`.
- Changed the items a batch inserts to draw their public ids in batch order.
- Changed a level of 0, an earlier draft's, to load as `Meta` with no summary, which a `Meta` upgrade revisits.
- Changed `PimdirStore::replace_action` to record the replaced intent's receipt.

### Removed

- Removed probes: `PimdirLevel::Probed`, `PimdirReader::count_probes`, `PimdirRekey::FETCH_CHUNK` and the probe statements.

  The `probes` table is dropped on open.

### Fixed

- Fixed a `Remove` on a pending create, or on the copy the hub offers, leaving a binding until a sync.

  It now withdraws it in the same write, the item retained or its delete pushed by the other sources (pimdir SYNC §7).

- Fixed a copy or move into a collection already holding its identity, staged under a minted key, never being landed by its arrival and being pushed a second time.
- Fixed a move of a body-less member into a collection already holding its identity deleting it from the server.

  The minted create now gets its origin, and the source's remove its destination with no `link_id`, so it relocates (pimdir SYNC §3, §4). A tombstone's destination now names the pending create's handle.

## [0.6.0] - 2026-10-03

### Added

- Added the `capability` module: the vocabulary, `PimdirSupport`, `PimdirCapability`, `required`, `required_by_content` and `check` (pimdir STORAGE §15.6, Annex B).
- Added `PimdirStore::declare`.

  The `capabilities` and `performers` tables are created on open in a store written by 0.5.

- Added `PimdirError::Unsupported`, returned by `PimdirProducer::enqueue` for an action a declared source does not support.

  A calendar write's resources are read for scheduling, occurrence changes and online meetings.

- Added `PimdirProducer::check`, returning the partial supports to show.
- Added performers: `PimdirAction::SetPerformer`, `PimdirProducer::performer`, `PimdirReader::performers`, `PimdirError::NoPerformer` and `PimdirError::Ambiguous`.

  A choice holds from the moment it is queued.

- Added `PimdirStore::replace_action`, acknowledging a performed intent with the store change it leaves, such as the copy of a sent message.
- Added `PimdirReader::capabilities` and `PimdirReader::item_capabilities`.
- Added `summary::calendar::scheduled`, `occurrences` and `online_meeting`.

### Changed

- Changed the drain to park an action a declared source does not support, naming the capability.
- Changed the drain to apply `set-performer`.

## [0.5.1] - 2026-10-01

### Changed

- Changed `vendored` to be on by default.

  `cargo install io-pimdir --features cli` builds SQLite from source and needs none on the machine. Use `--no-default-features` to link the system SQLite, as the Nix builds do.

### Fixed

- Fixed a server edit made while an item was in a cross-source conflict being dropped silently.

  The binding is now marked conflicted with the new revision, whatever the source's policy (pimdir SYNC §5).

- Fixed a `Remove` settling a conflict leaving a base that paired the old body with the new revision (SYNC §7).

  A later revival no longer reads the server as in sync.

## [0.5.0] - 2026-09-07

### Added

- Added the sync engine: `PimdirOpen`, `PimdirUpgrade`, `PimdirMutate`, `PimdirSync` and `PimdirRekey`, I/O-free coroutines in the core.
- Added `PimdirSourceStore::run`, `sync`, `upgrade`, `mutate`, `rekey`, `open_collection` and `service`, running the verbs against a consumer's `PimdirRemote`.
- Added typed summaries, derived by `summary::derive` (STORAGE Annex A).

  It yields the key, the kind's summary row with the people it names, and the sort key. The rows land in the five summary tables and `item_address`, and every listing read joins them (`list_summaries`, `get_item`, `address_placements`).

- Added the probes table.
- Added the change feed: `change_cursor`, `items_changed_since` and `collections_changed_since`.
- Added the refcount floor.
- Added every canonical statement the store lacked, so the schema is the specification's verbatim.
- Added the conformance suites: vectors/sync/ against the real store through a scripted connector, vectors/summaries.json against the derivations.
- Added `PimdirSync::beside_other_sources`, deciding a delete the source may not push (SYNC §5).

  It is held beside another source of the collection and reverted for a source alone. The std client answers it from `collection_sources`, the bindings and checkpoints of the collection.

- Added the re-staging of a local edit whose member the server deleted (SYNC §5).

  Its handle drops as `Superseded` and the edit becomes a pending create under its provisional handle, never dropped.

- Added `PimdirLinkId::provisional`, the `U+0001` handle namespace every staged create sits under.
- Added `PimdirMutateError::Undeliverable`, for a copy or move of a placement with neither a body nor a based binding (SYNC §3, §7).
- Added the item-level conflict to the projection (SYNC §3, §7, STORAGE §14.1).

  Every binding of a conflicted item projects `Conflict` with no revision, an `Edit` or a `Remove` settles it, and `list_item_conflicts` lists what waits.

- Added `PimdirSourceStore::delete_collection`.
- Added `list_pending_actions`, store-wide in append order.
- Added `PimdirProducer::MAX_ATTEMPTS`.
- Added `PimdirError::InFlight`, returned by a collect while a verb of this process runs.
- Added the landing of a pending create by its arrival (SYNC §6).

  An upgrade whose fetch resolves the hint a pending create holds supersedes the provisional handle instead of minting a second copy.

- Added the destination of a `Tombstone` placement (`DESTINATION_FOR_LINK`).

  A move's remove relocates through the reference store.

- Added `LOAD_PROBES_BY_HANDLE`, for a `Handles` load.
- Added `sql::MIGRATIONS` and the migration runner on the owner open (STORAGE §6).
- Added `PimdirRekey::FETCH_CHUNK`, bounding the `Meta` fetches a rekey resolves the new spine in.
- Added the per-store writer lock, serialising the collector against the process's own writers.
- Added the `json-schema` subcommand, listing the JSON Schema of every command's `--json` output.
- Added hidden plural aliases on the CLI verbs: `completions`, `manuals`, `collections`, `items`, `queues` and `stores`.
- Added the release workflows and the Nix derivation behind them.

  A tag ships a `pimdir` binary per platform with its manual pages, completion scripts and JSON Schemas, and publishes the crate. The binaries carry `cli` and `vendored`, each holding its own SQLite.

- Added `PimdirSourceStore::set_collection_name` and the canonical `set_collection_name` statement (STORAGE §14).

  It sets a collection's readable name, otherwise seeded from its id.

### Changed

- **BREAKING**: Renamed every engine type from `Replica*` to `Pimdir*`.

  For example, `ReplicaSourceBinding` is `PimdirBinding` and `ReplicaEvent` is `PimdirSyncEvent`.

- **BREAKING**: Renamed the read placement to `PimdirItemLocation`.
- **BREAKING**: Moved the client types under `client::{reader, producer, blobs, diagnostics}`, with no crate-root re-export.
- **BREAKING**: Renamed the `conventions` module to `summary`.
- **BREAKING**: Changed `PimdirDerivation` to carry a typed summary instead of a meta string.
- **BREAKING**: Changed every collection parameter of the std client to take `impl AsRef<str>`.

  `PimdirCollectionId` implements `AsRef<str>`.

- **BREAKING**: Changed `PimdirProducer::enqueue` to take no timestamp, and the body as `Option<&PimdirObject>` instead of a size.
- **BREAKING**: Changed `PimdirAction::Add` and `Update` to carry no summary.

  The owner derives it from the body.

- **BREAKING**: Changed the change key to the derivation SYNC §4 fixes, so every key changed.

  It is FNV-1a 64 over NUL-terminated fields, with a flag count in decimal ASCII.

- **BREAKING**: Renamed the coroutine states to the present tense.
- **BREAKING**: Changed a completed coroutine to answer `UnexpectedArg` to every resume, `None` included.
- **BREAKING**: Replaced `sql::ALL` with `sql::CANONICAL`, `sql::OWN` and `sql::all()`.
- **BREAKING**: Changed every store from an earlier draft to be refused with `PimdirError::Stale` and recreated.

  `PimdirError::Stale` carries the `missing` table or trigger. A store lacking a table is `Stale` before its stamp is read.

- **BREAKING**: Renamed every `--json` key of the CLI output to camelCase.
- **BREAKING**: Renamed the `completions` and `manuals` commands to `completion` and `manual`.
- **BREAKING**: Replaced the `schema_version` and `supported_schema_version` pair of `store info` with one `schemaVersion`.
- **BREAKING**: Changed `PimdirMutation::Copy` and `Move` to carry no `placeholder`.

  Every create sits under the link id's provisional handle.

- **BREAKING**: Changed `PimdirMutation::Add` and `PimdirAction::Add` to carry no `handle`.
- **BREAKING**: Changed `PimdirArg::LookupObject` and `lookup_objects` to answer `PimdirObject`s, hash and size (SYNC §6).
- **BREAKING**: Renamed `drain_collection` to `drain`, store-wide in append order (STORAGE §15.2).
- **BREAKING**: Changed `PimdirPendingAction` to name its `collection`.
- **BREAKING**: Changed `PimdirDrainReport` to count `retried`.
- **BREAKING**: Changed `PimdirChangeCursor::changed` to the last stamp drawn rather than the next.
- **BREAKING**: Changed `PimdirRetention::at` to `Option<String>`, `None` for a tombstone a source still holds.
- **BREAKING**: Changed `rename_collection` to take `&mut self` and retarget the queued moves and copies (STORAGE §4.5, §11, §15).
- **BREAKING**: Changed the `client` feature to link the system SQLite through pkg-config.

  The new `vendored` feature, off by default, restores the source build (`rusqlite/bundled`).

- Changed the canonical SQL to pimdir draft-01's, vendored under spec/ byte for byte, its constants generated by build.rs, one per statement file.
- Changed a drop to carry one of three reasons: `Deleted`, `Superseded` and `Rekeyed`.

  `Superseded` covers an accepted add's provisional handle, one a landed arrival replaced, or the handle of a vanished edit re-staged. Only `Rekeyed`, a rebuild, bumps the collection's generation.

- Changed a sync's events to report only what the remote changed and an accepted add.

  A pushed flag, body or delete reports nothing.

- Changed a write to retire the binding a handle held under another link id before inserting the new one (STORAGE §10).
- Changed a write to purge a dropped item's row, instead of retaining it, when another collection of the same account holds the identity live (STORAGE §11).
- Changed a load by key (`PimdirLoadScope::Links`) to answer the rows the source binds only, never the copy the hub offers for an item the source lacks.
- Changed a rekey to carry a remote edit as a pull, a conflict over a local edit or a revived tombstone, never a base at a revision it never reconciled (SYNC §8).

  It lists every `Rekeyed` drop before every upsert, pairs the copies of one hint by base revision then body, and carries an item-level conflict with no revision.

- Changed a create to wait while the collection holds a probe.
- Changed a `Full` fetch to move the base, mark a conflict over a staged edit the remote moved past, and clear a conflict whose diverging body is the placement's own.
- Changed a mutable member restating its hint to be keyed afresh.
- Changed a tombstone holding a staged edit over a remote edit to follow the conflict policy.
- Changed a `Remove` to settle a conflict against the recorded revision.
- Changed an immutable kind's base to adopt the shared body.
- Changed the hub to judge mutability across every binding.
- Changed the hub to read the divergence against the base the binding held.
- Changed the hub to keep a rebuilt binding's base for the upsert that rebinds it.
- Changed the hub to adopt a tombstone's known flags.
- Changed the hub to lower the shared level with a body a pull drops.
- Changed the hub to ask its resolver for `Tombstone` placements too.
- Changed `PimdirMutate` to refuse a probe with `PimdirMutateError::Probed`.
- Made `client::PimdirRunError` public.
- Changed a drain to park a store failure, and a move or copy into a collection of no declared kind.
- Changed a drain to retry a busy or I/O failure up to `MAX_ATTEMPTS` without stopping the pass.
- Changed a drain to apply each action as a source binding its item or syncing its collection, rather than as the handle draining (STORAGE §15.2).
- Changed `enqueue` to pin the body before the row lands (STORAGE §15.1).
- Changed `collect_garbage` to recompute the refcounts before deleting (STORAGE §11).
- Changed `held_elsewhere` to compare bodies.
- Changed a revive to keep a retained body a `Meta` fetch carries none of.
- Changed the trash to list every deleted row.
- Changed the schema check to cover the triggers (STORAGE §5).
- Changed the collector to unlink only files at their shard path.
- Changed a blob write to sync every shard directory it created.
- Changed a reader or producer opening a directory with no database to report `Uncreated`.
- Bumped `rust-version` to 1.88.
- Bumped the export dump `format_version` to 2.

  Items carry summary columns and address rows instead of a raw meta string.

- Changed the `cli` feature to pull pimalaya-cli through `dep:`.
- Changed the package to exclude .github/, cairn/ and tests/proptest-regressions/.

### Removed

- Removed `items.meta` and `ReplicaMeta`.
- Removed the draft reconcile (`FOLDED_IN`, `ENSURE_INDEXES`, `RESHAPED_INDEXES`) and `PimdirError::Unreconcilable`.
- Removed `PimdirStore::open_read_only`, superseded by `PimdirReader::open`.
- Removed the in-memory residual of unnamed handles, superseded by the probes table.
- Removed `write_rekeyed`, superseded by the `Rekeyed` drop reason.
- Removed `queued_collections`.
- Removed `PimdirConflictPolicy::KeepBoth` (SYNC §5).
- Removed `PimdirDeletePolicy` and `PimdirSyncOptions::delete` (SYNC §5).
- Removed the `io-replica` dependency.
- Removed the accidental public `pimalaya-cli` feature.
- Removed the duplicated `fs4` and `rusqlite` dev-dependencies.

### Fixed

- Fixed the `hash:` key using a wrong FNV-1a 64 prime.

  A UID-less card or event is now keyed as the specification says.

- Fixed an accepted content push losing a flag pulled in the same run (SYNC §5).
- Fixed a re-listed probe with unchanged flags being rewritten and reported on every full sync.
- Fixed a probe an enumeration no longer lists never being dropped.
- Fixed `PimdirSyncReport::rejected` counting a rejection for a handle nobody pushed.
- Fixed a bound source projecting dirty on every load after a sibling source pulled a content change.
- Fixed an `Edit` on a pending copy copying the origin's body instead of uploading the edited one.
- Fixed `windows-1252` being decoded as latin1.
- Fixed folded header whitespace being collapsed.
- Fixed an encoded word whose payload starts with `=` failing to decode.
- Fixed one malformed pending row failing every overlaid read.
- Fixed single-statement owner writes surfacing `SQLITE_BUSY` as a raw SQL error.
- Fixed unknown address roles reading as `from`.
- Fixed the CLI opening a second reader behind the handle a verb already held.
- Fixed CI checking out io-replica and running a `conventions` suite that no longer exists.
- Fixed the docs.rs feature badge missing on the `client` module.

## [0.4.1] - 2026-09-02

### Fixed

- Fixed a write with no link id filing a second row for a handle already bound to an item.

  Such an upsert now folds into the item its binding names. Only a handle no binding holds still stages unlinked.

## [0.4.0] - 2026-08-31

### Added

- Added `bindings.shared_object`, what a source last agreed with the hub on.

  A single-source store no longer files a second offline edit, or the edit resolving a conflict, as a cross-source conflict. It round-trips through `ReplicaSourceBinding`, is backfilled from the item's body on open, and `item show` prints it.

- Added `bindings.conflict_object`, the diverging body an unresolved conflict is waiting on.

  Base, local and remote are readable from the store alone. It round-trips through `ReplicaSourceBinding`, stays pinned while `conflicted` is set, is reconciled on open in an older store, and `item show` prints it.

- Added `PimdirReader::list_conflicts`, the outstanding conflicts across an account's collections.

  Each binding is named by its collection, link id, source and handle, with the three hashes it diverges between.

- Added `PimdirReader`, the read role (spec §8).

  It takes no lock and carries no write. `PimdirStore` dereferences to it.

- Added the pending-action overlay (spec §15.4).

  A reader built `with_pending` folds the queue's pending actions over the committed items, parked rows excepted. Queued creates have no `seq` yet and are reported by `pending_creates` and `count_pending_creates`.

- Added `PimdirStore::cancel_action`, cancelling one queue row as the owner for the length of the call only.
- Added `PimdirBlobs::path`, where a body under a hash lives.
- Added the `conventions` module, the per-kind derivations (spec Annex A).

  `derive(kind, body)` returns the `link_id`, `meta` summary and `sort_key` of `message/rfc822`, `text/vcard` and `text/calendar` in the core. Fallback ids are `alt:` for a message with no `Message-ID`, `hash:` for a card or resource with no `UID`.

- Added `pimdir gc`, the collector.

  It drops the object rows nothing references, unlinks their bodies and the orphan blob files a crash left, and reports what it freed. It takes the owner role.

- Added the single-owner lock (spec §8).

  An owning handle holds an exclusive advisory lock on the store directory, and a second owner process gets `PimdirError::Owned` at once. Readers take none, producers a shared one.

- Added `PimdirReader::item_bindings`, printed per source under each placement by `pimdir item show`.
- Added the minted-key count `pimdir check` reports per collection: the second copies of an identity a source hands over twice (spec §9).
- Added the indexes `objects_garbage`, `items_by_seq_global`, `bindings_by_handle`, `items_by_conflict_object` and `queue_by_object`.

### Changed

- **BREAKING**: Dropped the source parameter from `PimdirStore::open` and `open_read_only`.
- **BREAKING**: Moved the source-bound operations to `PimdirSourceStore`, yielded by `for_source`.

  It carries the storage seam, the rekeyed write and the queue drain, and dereferences to the store for everything else.

- **BREAKING**: Folded `PimdirRetainedItem` into `PimdirItem` as an optional `retention`.
- **BREAKING**: Changed `PimdirPlacement` to carry typed columns (`ReplicaLinkId`, `ReplicaFlags`, `ReplicaLevel`).
- **BREAKING**: Split `PimdirError::Version` into `Version`, a schema this crate does not service, and `Uncreated`, no schema yet.
- **BREAKING**: Folded the operator tool's read-only connection (`PimdirDb`) onto the store as a diagnostics block.
- **BREAKING**: Bumped the `io-replica` dependency to 0.5.

  `load` carries a scope, `DropPlacement` carries a reason, and `ReplicaCollection` is gone.

- **BREAKING**: Changed a purge to report the rows it retired, not the bytes it reclaimed.

  `PimdirPurgeReport` loses its `bytes` field and `item purge` its byte count.

- Changed `pimdir check --fix` to repair drifted refcounts and dangling bindings rather than delete orphan blob files.

  Orphan files are reported, and `pimdir gc` takes them.

- Changed `item restore` to queue rather than fail when the store is owned.

  It reports `queued`, and the owner holding the store drains it.

- Moved the blob write out of the database's writer lock.

  Bodies land in the blob store before the transaction indexing them opens (spec §14). The queue drain still writes inside its transaction.

- Changed the collector to check each file on the primary key (`OBJECT_EXISTS`) rather than holding every hash in memory.
- Changed a purge to release the pins its own delete reported.

  `PURGE_ITEM` and `PURGE_RETAINED_BEFORE` return each removed row's `object_hash` and `conflict_object`.

- Changed a write to read only the rows its batch names.

  Dropped handles resolve through `bindings_by_handle`, so a one-flag write no longer costs the size of the collection.

- Changed the residual of unnamed handles to be keyed rather than listed.
- Changed the drain to answer the `Add` collision check, the handle lookup and the mutation with point reads.
- Changed `release_pins` to one statement rather than one per hash.
- Changed the object sweep to collect `refcount <= 0`, so a count driven negative no longer leaks.
- Changed the fidelity suite to prepare every canonical statement against the canonical schema, and checked object naming against vectors/objects.json (spec §16).

### Deprecated

- Deprecated `open_read_only` in favour of `PimdirReader`.

### Removed

- Removed `RETAINED_ITEM_BY_SEQ` and `RETAINED_BEFORE`.
- Removed the `--grace` flag and the confirmation prompt of `pimdir check --fix`.

### Fixed

- Fixed a queued action the draining source could not place being parked, destroying it for the source that could.

  Such a row is now left pending, `attempts` untouched, and counted as `skipped`. Rows already parked stay parked, `queue cancel` remaining the only exit.

- Fixed a trash page sorting the whole trash to return fifty rows.

  `items_retained` now follows `list_retained_page` onto `seq`. On open, an index whose columns moved is dropped and recreated.

- Fixed a process refusing itself, with `PimdirError::Owned`, the store it had just released.
- Fixed a body stored without a placement being destroyed at the end of the batch.

  Writes no longer collect unreferenced objects. `pimdir gc` does.

- Fixed a body lookup crossing accounts.

  `lookup_objects` now resolves a link id within the caller's own account, still across its collections.

- Fixed a base of no revision, no body and unread markers round-tripping as no base at all.

  `bindings.base_present` now records it, so the sync no longer re-derives the same push on every run.

- Fixed an identity a source held twice being stored once, the write that lost the other silent.

  The second copy is an item of its own under the key the engine mints (spec §9). A write resolving a stored binding to another handle is refused with `PimdirError::Rebind`, a handle-space rebuild excepted.

- Fixed a write carrying a new sort key silently discarding it.
- Fixed a descending page hiding every item sorting above its first cursor.

  The first page now takes a `NULL` cursor rather than a sentinel key.

- Fixed two owners draining one collection applying every action twice.

  The drain claims a row with `CLAIM_ACTION` as its transaction's first statement, and a claim that deletes nothing skips the row.

- Fixed a blob rename never being made durable.

  The directory entry is now synced, so a power loss cannot leave a committed row without its body.

- Fixed a flag set the store could not decode reading as a known-empty one.

  It now decodes as unknown, so the merge no longer clears every marker the other side reported.

- Fixed `created_at` holding epoch milliseconds, or the empty string, instead of an RFC 3339 timestamp.

  SQLite writes it now, in the form the retirement clock uses.

## [0.3.0] - 2026-08-24

### Added

- Added `PimdirHashAlgo`, the content hash a store's objects are named by.

  It implements `blake3` (recommended) and `sha256-128` in lowercase base32 (RFC 4648, no padding), whole (`hash`) or incremental (`hasher`). A store, a producer and the algorithm itself hand it out.

- Added `PimdirStore::open_with_hash` and `PimdirError::HashAlgo`.

  `open_with_hash` declares the algorithm a store is created with and `open` adopts the recorded one. An open declaring a different one fails with `PimdirError::HashAlgo`.

- Added `PimdirStore::blobs`, a blob handle already bound to its store.
- Added `PimdirError::Unreconcilable`, refusing on open a store created before the rename cascades.

  Every store created by 0.2.0 is refused and must be recreated.

- Added `PimdirError::VersionMismatch`, refusing a store whose `PRAGMA user_version` and `store_meta.version` disagree (spec §4.2).

  Both the owner and the read-only open check it. A store with no `store_meta` row is left alone.

- Added `list_items_page_asc` and `list_items_page_desc`, keyset pages in a kind's own order.

  They order by the new `items.sort_key` column and `items_by_sort` index, with a `(sort_key, seq)` cursor the first page omits. An empty key means unknown: last newest-first, first A to Z.

- Added `set_sort_key`, restating one item's sort key.
- Added `collections.account`, the account a collection belongs to.

  It comes with the partial `collections_by_account` index (spec §9.2) and is reconciled on open in an older store. It partitions nothing: link ids, object hashes and `seq`s keep their store-wide meaning.

- Added `for_account` on `PimdirStore` and `PimdirProducer`, binding a handle to one account.
- Added `list_accounts`, `list_collections_by_account`, `collection_account` and `set_collection_account`.

  `list_accounts` reports what the collections say: there is no `accounts` table.

- Added `link_placements` and `object_placements`, every live `PimdirPlacement` of an identity or a body with its collection and account.
- Added `rename_collection`, carrying a collection's items, bindings, sources, queue rows and child collections to a new id.

  Every foreign key onto `collections(id)`, and `bindings(collection, link_id)`, is now `ON UPDATE CASCADE`.

- Added the spec-fidelity test suite against the canonical pimdir specification checked out beside the crate.

  It compares the schema semantically, checks every canonical statement is present by name and prepares every inlined statement. It skips when the specification is absent.

### Changed

- **BREAKING**: Changed `PimdirBlobs::open` to take the hash algorithm.
- **BREAKING**: Changed `ItemRow.flags` to `Option<Vec<String>>`, `null` in the JSON output while nothing has read the markers.
- **BREAKING**: Changed `PimdirItem` and `PimdirRetainedItem` to carry a `sort_key` field.
- Changed an unknown flag set to be written as `NULL` rather than `'[]'` (spec §13).

  `flags_to_json` returns `None` for `ReplicaFlags::Unknown` and `flags_from_json` decodes `NULL` back to it. A queue payload encodes it as `null` (§15.3).

- Changed the sort key io-replica carries on a placement to be bound on write.

  `load` returns it, and insert and update write it.

### Fixed

- Fixed an earlier-draft store failing every paged read with `no such column: sort_key`.

  The draft reconciliation on open now adds `items.sort_key` and the `items_by_sort` index.

- Fixed the inlined schema drifting from the specification.

  `sql::MIGRATION_0001` now carries the `sort_key` column and every `ON UPDATE CASCADE`.

## [0.2.0] - 2026-08-07

### Added

- Added the `pimdir` operator CLI, behind the `cli` feature.

  It never interprets item content and renders JSON under `--json`, logs on stderr. Reads open the store read-only, so they are safe while a sync runs, and listings never truncate silently.

- Added the `collection list`, `item list`, `item show`, `item export`, `store info` and `export` commands.

  `item list` is keyset-paged and lists retained items with `--retained`. Bodies export byte for byte.

- Added the `item restore` command, queueing the action and draining it as the owner.

  It reads the item back to report it applied. When a sync holds the lock, the action stays queued for the next drain.

- Added the `item purge` command, taking one `seq`, `--older-than <DURATION>` or `--all`.
- Added the `queue list [--parked]` and `queue cancel` commands.
- Added the `check [--fix]` command.

  It reports orphan blob files and refcount or reference drift. `--fix` reclaims only orphan files, guarded by a `--grace` window.

- Added the `completions` and `manuals` commands.
- Added the confirmation of destructive verbs, skipped with `--yes`.

  They refuse to prompt into a pipe or under `--json`.

- Added item retention: an item whose last source binding vanishes is soft-deleted.

  `items` gains `retained_at`, `retained_by` and the partial `items_retained` index, and the row keeps its body pinned. Retention is unconditional, and `LOAD_ITEMS` hides retained rows from the sync seam.

- Added `purge(collection, seq)` and `purge_retained_before(cutoff)`, the only true deletes.

  Both release the row's object pin and refuse to touch a live item. The cutoff is an RFC 3339 instant.

- Added `list_retained`, `count_retained` and `retained_bytes`, the trash view.

  `retained_bytes` is an upper bound on what a purge would reclaim.

- Added the revival of a retained row by a reappearing link id.

  It clears `deleted`, `retained_at` and `retained_by`, adopts the new content and keeps the `seq`. Restoring an item is an ordinary `Add`.

- Added `PimdirAction::Unknown`, an unrecognised action kind carried verbatim with its body pinned.

  The drain leaves such a row pending, counted in `PimdirDrainReport.skipped`, without blocking the actions behind it. Malformed payloads still park.

- Added `drop_action(id)`, removing one queue row, pending or parked, with its object pin.
- Added `fail_action(id, error)`, bumping `attempts` for a transient failure or parking with the reason for a permanent one.
- Added `bindings.conflicted` and `bindings.conflict_revision`, the per-source content conflict.

  They round-trip through `ReplicaSourceBinding`, apart from the item-level `conflicted` and `conflict_object`. The revision is meaningful only while conflicted (spec §11).

- Added the reconciliation on open of a store written by an earlier draft of schema version 1.

  `init_schema` adds any folded-in column it finds missing, and any index over one (spec §6's draft allowance).

### Changed

- **BREAKING**: Bumped io-replica to 0.3.
- Changed `PimdirAction::kind()` to return `&str` rather than `&'static str`.

### Removed

- Removed `PimdirActionError::UnknownKind`.
- Removed `sql::DELETE_ITEM`.

  An item no source holds is retained (`sql::RETAIN_ITEM`). `sql::PURGE_ITEM` and `sql::PURGE_RETAINED_BEFORE` are the only true deletes.

## [0.1.0] - 2026-08-06

### Added

- Added the pimdir store: a SQLite index plus a content-addressed, two-level-sharded blob directory.

  It implements io-replica's storage seam (`load`, `lookup_objects`, `write`) for one source.

- Added the no_std core, reusable without the SQLite client: the canonical schema and statements (`sql`) and the model-to-column encodings (`codec`).
- Added store-global public ids (`seq`): one per message, shared across every collection it is filed in, monotonic and never reused.
- Added streaming blob ingest and read, so a large body is never held whole.
- Added the byteless object write, indexing a body already streamed to its content-addressed path.
- Added incremental, cross-collection reference counting with blob garbage collection inside the write transaction.

  A crash leaves at worst an orphan blob, never a row without its body.

- Added single-writer serialisation through `BEGIN IMMEDIATE` and a generous busy timeout.

  Several same-source handles overlap network while their writes serialise.

- Added the availability-aware, paginated read surface: `list_items`, `get_item`, `count_items`, `distinct_sources` and `seq_for_link`.
- Added the action queue table and `collections.generation` to the draft v1 schema.
- Added the schema version checks on open: `user_version` and `store_meta.version` kept in agreement, and a store stamped with a higher version refused.

  Draft stores are recreated, never migrated.

- Added `PimdirProducer`, the single enqueue transaction any non-owner process may run (spec §14).

  It pins a pre-written body against garbage collection.

- Added the owner's drain, `drain_collection`, applying each action and deleting its row in one transaction.

  Permanently failing actions are parked.

- Added `queued_collections`, `pending_actions` (the read-your-writes overlay) and `parked_actions`.
- Added the action payload codec in the no_std core: `PimdirAction` (add, set-flags, remove, move, copy, update), addressing items by public `seq`, with a strict, versioned JSON round-trip.
- Added collection generations (spec §15): the handle-space epoch on `PimdirCollection` and `generation()`, bumped atomically with a rebuild batch by `write_rekeyed()`.
- Added the read-only open, `open_read_only`.

  It opens an existing store with `SQLITE_OPEN_READ_ONLY`, never creates anything, refuses any other schema version and exposes the full read surface.

[unreleased]: https://github.com/pimalaya/io-pimdir/compare/v0.20.4..HEAD
[0.20.4]: https://github.com/pimalaya/io-pimdir/compare/v0.20.3..v0.20.4
[0.20.3]: https://github.com/pimalaya/io-pimdir/compare/v0.20.2..v0.20.3
[0.20.2]: https://github.com/pimalaya/io-pimdir/compare/v0.20.1..v0.20.2
[0.20.1]: https://github.com/pimalaya/io-pimdir/compare/v0.20.0..v0.20.1
[0.20.0]: https://github.com/pimalaya/io-pimdir/compare/v0.19.1..v0.20.0
[0.19.1]: https://github.com/pimalaya/io-pimdir/compare/v0.19.0..v0.19.1
[0.19.0]: https://github.com/pimalaya/io-pimdir/compare/v0.18.1..v0.19.0
[0.18.1]: https://github.com/pimalaya/io-pimdir/compare/v0.18.0..v0.18.1
[0.18.0]: https://github.com/pimalaya/io-pimdir/compare/v0.17.0..v0.18.0
[0.17.0]: https://github.com/pimalaya/io-pimdir/compare/v0.16.0..v0.17.0
[0.16.0]: https://github.com/pimalaya/io-pimdir/compare/v0.15.1..v0.16.0
[0.15.1]: https://github.com/pimalaya/io-pimdir/compare/v0.15.0..v0.15.1
[0.15.0]: https://github.com/pimalaya/io-pimdir/compare/v0.14.1..v0.15.0
[0.14.1]: https://github.com/pimalaya/io-pimdir/compare/v0.14.0..v0.14.1
[0.14.0]: https://github.com/pimalaya/io-pimdir/compare/v0.13.0..v0.14.0
[0.13.0]: https://github.com/pimalaya/io-pimdir/compare/v0.12.0..v0.13.0
[0.12.0]: https://github.com/pimalaya/io-pimdir/compare/v0.11.0..v0.12.0
[0.11.0]: https://github.com/pimalaya/io-pimdir/compare/v0.10.0..v0.11.0
[0.10.0]: https://github.com/pimalaya/io-pimdir/compare/v0.9.0..v0.10.0
[0.9.0]: https://github.com/pimalaya/io-pimdir/compare/v0.8.0..v0.9.0
[0.8.0]: https://github.com/pimalaya/io-pimdir/compare/v0.7.0..v0.8.0
[0.7.0]: https://github.com/pimalaya/io-pimdir/compare/v0.6.0..v0.7.0
[0.6.0]: https://github.com/pimalaya/io-pimdir/compare/v0.5.1..v0.6.0
[0.5.1]: https://github.com/pimalaya/io-pimdir/compare/v0.5.0..v0.5.1
[0.5.0]: https://github.com/pimalaya/io-pimdir/compare/v0.4.1..v0.5.0
[0.4.1]: https://github.com/pimalaya/io-pimdir/compare/v0.4.0..v0.4.1
[0.4.0]: https://github.com/pimalaya/io-pimdir/compare/v0.3.0..v0.4.0
[0.3.0]: https://github.com/pimalaya/io-pimdir/compare/v0.2.0..v0.3.0
[0.2.0]: https://github.com/pimalaya/io-pimdir/compare/v0.1.0..v0.2.0
[0.1.0]: https://github.com/pimalaya/io-pimdir/compare/root..v0.1.0
