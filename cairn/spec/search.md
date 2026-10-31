---
cairn: spec
capability: search
status: current
---

# Search

The reference implementation of pimdir SEARCH.md (`search-draft-07`), experimental: its API and index build may change in a minor release: index.db beside the store, its refresh from the change feed, extraction and the query language. It ships behind the `search` feature, which `cli` enables and the default leaves out, so a consumer that does not search carries none of it. `search` alone is I/O-free (src/search/: the query language, the folds, extraction, chunks, occurrences, tags, threads and the refresh coroutine); `search` with `client` adds `PimdirIndex`, index.db over rusqlite (src/client/search.rs). Milestones M1 to M4 of pimdir's search-milestones: every kind's extraction, every field, occurrences within the horizon, threads, relative dates, tagging and the thread verb.

### Requirement: The index is derived and rebuilt, never migrated
`PimdirIndex::open` SHALL open index.db as the main database with pimdir.db attached read-only as `store`, and SHALL rebuild it when its version, its tokenizer or its shape differs from the vendored schema's (SEARCH §2): every table, index and trigger migrations/search/0001_init.sql creates, compared by its `sqlite_schema` text with `--` comments removed and whitespace collapsed, the shadow tables FTS5 creates and any other table left out. An index an earlier draft or an earlier release of this crate built is therefore rebuilt rather than answering from data it lacks, and stored chunk offsets are always this crate's extraction's, which a snippet re-extracts (SEARCH §6). `PimdirIndex::open_query` SHALL refuse such an index as `PimdirError::IndexOutdated`. It SHALL refuse a SQLite without FTS5 or older than 3.43 (`PimdirError::Fts5`) rather than build a partial index.

#### Scenario: An index an earlier draft built
- GIVEN an index.db at version 1 whose `object` table lacks `attachment`
- WHEN `open_query` opens it, THEN it answers `IndexOutdated`; WHEN `open` opens it, THEN it rebuilds it and the next `open_query` opens it (tests/search.rs `an_index_of_another_shape_is_rebuilt_never_queried`)

### Requirement: A refresh folds the feed in bounded, resumable batches
`PimdirIndex::refresh` SHALL take index.lock without waiting, answering `busy` when another indexer holds it. It SHALL fold `load_changed_items` in transactions of a bounded number of rows, each one run of `PimdirIndexRefresh` answered through the canonical statements (src/client/search/refresh.rs), each full batch recording through `set_index_cursor` its last row's stamp with the purge count and `store_reconciled` the last completed pass recorded, so a pass cut short resumes where it stopped. The transaction exhausting the feed SHALL reconcile keys when the purge count moved or a collection was stamped above `index_meta.store_reconciled`, and record the cursor read before the pass as `store_change` and `store_reconciled`, with the purge count read beside it: no batch advances `store_reconciled`, so a rename a deadline-cut pass folded past is still reconciled, by this indexer or another one resuming the pass. A deadline SHALL be checked between batches. A handle opened by `open_query` SHALL refuse to refresh (`PimdirError::IndexQueryOnly`).

### Requirement: One-shot answers refresh first and say what is behind
`search` and `count` SHALL refresh up to the store's head within `--refresh-timeout` before answering (`0s` skips it), and SHALL count what stays unfolded as `coverage.local.unindexed`; every answer SHALL say in `coverage.local.behind` (`behind` in JSON) what the canonical `behind` answers: the change stamps the store drew above the index cursor, store-wide. A session that keeps the index open follows the feed instead.

### Requirement: Mail is extracted by mail-parser, chunked as SEARCH §6 says
Mail extraction SHALL go through mail-parser (`full_encoding`): the MIME walk under `MIME_DEPTH_MAX`, transfer encodings, charsets, RFC 2047 and 2231, `text/plain` preferred and `text/html` reduced to text, `encrypted` and `unparseable` statuses, the attachment field from the crate's own stand-in walk. The `body` and `note` fields SHALL be cut into chunks by `search::chunk::chunks` on SEARCH §6's rule. Contacts and calendar items SHALL be extracted through vcard-rs and ical-rs on SEARCH §6's table, a `TEL` as written and as its digits, every event, task and journal of a resource, overrides included; an item falls back to its summary only while bodiless or unparseable, and a file always reads its summary.

### Requirement: Calendar time is expanded through ical-rs within a rolling horizon
Occurrences SHALL be expanded by ical-rs's `IcalRecurTimeline` with zones from `ical::tzdb::zones`, so the resource's own `VTIMEZONE` wins over the bundled database and an unknown `TZID` reads floating, within a horizon of one year back and two forward around the refresh's `now`, at most `OCCURRENCES_MAX` per item. An item SHALL be re-expanded when its body changes, when the horizon has rolled by a day or more, and when `ical::tzdb::version()` differs from the recorded one, which `set_tzdb_version` then records. A resource holding overrides and no master yields its overrides. `date:` and `when:` SHALL compare a bodied calendar item's occurrences, and coverage SHALL report `horizonLeft` when a range leaves the horizon with a calendar collection in scope.

### Requirement: A query is parsed whole or refused with SEARCH §8.4's object
`PimdirQuery::parse` SHALL accept exactly SEARCH §8.3's grammar and refuse anything else with a `PimdirQueryError` carrying §8.4's code and span, never reading an unknown field as text. On `unknown-field` its `expected` SHALL be empty, `fields` carrying the list, and a name that lowercases to a field SHALL be named in the message. A statement failing on a query the grammar accepted SHALL answer `PimdirError::Unanswerable`, never a raw SQL error. The AST SHALL be public, so a consumer can compile its own filter into it.

### Requirement: Every answer carries its coverage
A search answer SHALL carry a `PimdirSearchCoverage`: the local axis (bodiless mail, contacts and calendar items in scope, never files, and the items not yet folded), the temporal axis (`Full`, or `Bounded` by the collections' coverage rows), and the remote axis (`LocalOnly` unless a consumer merged a server answer). The roll-up keeps the worst of each axis. `PimdirReader::search_coverage` SHALL fill the same type for the store's `LIKE` reads, which stay for consumers built without the feature.

### Requirement: Hits are addressable and typed
A hit SHALL be `(account, seq)` with its placements, its kind and title, what it matched (`body`, `summary`, or none when only structured predicates matched) and a snippet cut from its chunks. The CLI's `--json` output SHALL use camelCase keys and publish its JSON Schema (`pimdir-index`, `pimdir-search`, `pimdir-count`, `pimdir-show`, `pimdir-search-threads`, `pimdir-thread`, `pimdir-tag`, `pimdir-query-error`), and `show SEQ --account ACCOUNT` SHALL accept a hit's address as is.

### Requirement: The vectors are the bar
tests/search.rs SHALL run every case of pimdir vectors/search (and vectors/search/chunks.json) against a store built as an owner leaves it, skipping when the spec checkout is absent. Every case SHALL run: none is skipped, `threads` cases included.

### Requirement: Threads are relinked by the refresh and listed on request
The refresh SHALL relink the messages a batch touched (SEARCH §9): the parent from the held body's first `In-Reply-To` id, else its last `References` id, or from `mail_summary.in_reply_to` for a bodiless item; a `message` row for every parent named, present or not; a cycle broken by ignoring the edge that would close it; the thread id the present member with the lowest `(sort_key, link_id)`, its new row written and members repointed before an unused row goes. `thread:` SHALL accept the thread id or any message id in it, so an id renamed by a join still resolves. `PimdirIndex::threads` (CLI `search --threads`) SHALL group hits by account and thread, in the order of each thread's first hit, `count`, `first` and `last` covering the thread's live members in that account; an item that is not mail stands alone.

### Requirement: Relative dates resolve against a clock
`today`, `yesterday` and relative amounts SHALL resolve in UTC against `PimdirSearchClock`, the system clock unless one is given, as SEARCH §8.1 states: a relative amount alone spans from its instant to now, a relative point as a range side is its instant, and a span reaching before year 0 is clamped to it.

### Requirement: Tagging stages through the queue, each placement as its own source
`PimdirIndex::tag` SHALL plan a flag change over a query's hits without writing: per placement its next flag set, starting from the last pending `set-flags` of that placement so tags staged in a row compose, `read`, `flagged`, `answered` and `draft` naming their system flags, `unread` and `encrypted` refused. Its source SHALL be resolved per placement from the sources syncing its collection: the requested one when it syncs it, else the only one; several and none requested is `ambiguousSource`, a requested one not syncing it is `unsynced`, and a mail source whose declaration lacks a capability the change needs is `unsupported`, naming what is `missing`; none of these is staged, and the other placements are. `stage` SHALL stage one `set-flags` per placement that changes and resolves, after every action passes the producer's check, so a refusal leaves the queue untouched. Each placement SHALL say whether its source can carry the change by its declared capabilities (`syncs`): a tag on a contact or calendar placement, or on mail whose source declares no matching capability, is kept in the store only. The CLI `tag` SHALL drain when the owner role is free and read each outcome back from the store (`applied`, `queued`, `unchanged`, `refused`, ...); `--dry-run` writes nothing.

### Requirement: An item's thread is answered from the index
`PimdirIndex::thread(account, seq)` SHALL answer the item's thread through `thread_of` and `thread_members`, members oldest first, with its id, count, first and last; an item that is not mail, or not threaded, answers itself alone; none when no live placement of it is in the account.

### Requirement: Tasks are searched by status, priority and category
`status:`, `priority:` and `category:` SHALL be parsed (`PimdirQueryPredicate::Status`, `Priority` with `PimdirQueryPriority`, `Category`) and compiled to `store.task_summary` (SEARCH §8): `status:` RFC 5545's `VTODO` set, lowercased, against the uppercased column; `priority:` `high` 1 to 4, `medium` 5, `low` 6 to 9, `none` absent or 0; `category:` one value whole, both sides folded as STORAGE Annex A.2 folds `fn` (the Unicode default full lowercase mapping, then trimmed, `search::fold::value`), in Rust: the store's distinct `task_category` values are read once per query naming a category (`TASK_CATEGORY_VALUES`), folded, and those folding to the term's compared as stated (`s.value IN (...)`), a card's against `object_category`, folded at extraction. No SQL function is registered: SQLite's `lower()` folds ASCII alone, and a shape on the platform's SQLite may register none. The first two refuse any other value as `invalid-value`, and `category:` takes no wildcard.

#### Scenario: The task vectors
- tests/search.rs, the eleven cases over item 16 of pimdir vectors/search/queries.json; the unit test `a_category_folds_by_the_full_unicode_lowercase_mapping` of src/search/fold.rs.

### Requirement: A calendar item has an attachment when its field is not empty
The calendar extraction SHALL put each `ATTACH`'s `FILENAME` (or `X-FILENAME`) and `FMTTYPE` in the `attachment` field, `insert_object` SHALL record in `object.attachment` whether that text is non-empty, and `has:attachment` SHALL match a message whose `mail_summary.attachment` is 1 or a calendar item `attached` names (SEARCH §6, §8). No implementation function reads FTS5's shadow tables.

#### Scenario: The attachment vectors
- the search vectors `has:attachment` and `has:attachment kind:task` over items 13, 16 and 23 (tests/search.rs)

### Requirement: A query client opens the index read-only and builds nothing
`PimdirIndex::open_query` SHALL open index.db read-only with pimdir.db attached read-only, take no lock and write nothing, the journal mode included (SEARCH §3): it never creates, rebuilds nor refreshes the index, which the store's indexer (its owner, at sync time) keeps. An index.db absent or holding no schema SHALL be `PimdirError::IndexMissing` (`index-missing`), one at another schema version, tokenizer or build `PimdirError::IndexOutdated` (`index-outdated`), each saying to run the store's sync engine. `PimdirIndex::open` stays the indexer's open, building and rebuilding.

#### Scenario: A missing, a lagging and an outdated index
- tests/search.rs `a_query_open_neither_builds_rebuilds_nor_refreshes`

### Requirement: A collection term names an id
`collection:` and `folder:` SHALL match collection ids alone, in the compiled query and in the coverage's scope: a collection's name is a label, never an address (STORAGE §14).

#### Scenario: A display name
- tests/search.rs `a_collection_term_names_an_id_never_a_display_name` (failed before the change: the name matched)

### Requirement: A value of digits names a seq and a link id
`id:` and `linked:` with a value of ASCII digits alone (`PimdirQueryId::Digits`, the text as written) SHALL name the item of that `seq` and every item whose link id it is (SEARCH §8), a hit set; a value past a `seq`'s range names the link id alone. `linked:` SHALL read the text before the first colon as a role only when it is one of STORAGE §14.2's or `x-` and a name.

#### Scenario: A message keyed by digits
- tests/search.rs `an_id_of_digits_names_the_seq_and_the_link_id` (failed before the change: `id:777` named seq 777 alone); src/search/query.rs `a_linked_value_reads_a_role_only_when_it_names_one`

### Requirement: A rename past a cut pass is reconciled
A collection renamed while a pass was cut by its deadline after folding past the rename's stamp SHALL be reconciled by the next completed pass, its old id's placements dropped.

#### Scenario: A cut pass
- tests/search.rs `a_rename_a_cut_pass_went_past_is_reconciled_by_the_next_one` (failed before the change: nothing reconciled)

### Requirement: A search answers at the scale of a mailbox
A refresh SHALL read one item's summary and addresses by seek (`get_mail` and its siblings by `(collection, seq)`, `load_addresses`), never by a scan of its collection, and a compiled term's membership SHALL never be `NULL`, so a negated term is one probe per hit; a page reads its hits' titles for the `name` order alone. Over 50,000 summary-only messages in one collection (tests/search_scale.rs, release build) the index builds in about 5 s, and `is:unread` answers a page of 50 in about 120 ms and a count in about 25 ms.

#### Scenario: 50,000 messages
- GIVEN 50,000 summary-only messages in one collection, a third read
- WHEN the index is built, then `is:unread`, `not is:unread`, `is:flagged`, `date:`, `from:`, `subject:` are asked a page and a count
- THEN the build takes about 5 s (454 s before) and every page and count under 0.2 s (`is:unread` 25 s before)

### Requirement: The index reads an older store as its reconciliation would leave it
Every statement the index runs naming the attached store (its compiled query, its feed, its reconciliation, its extraction, its thread and occurrence reads) SHALL go through `schema::StoreShape` (STORAGE §6): unchanged on a store holding every canonical table and column, else read as its reconciliation would leave it, so a store an older release wrote is indexed and searched before its owner reconciles it.

#### Scenario: Searching an older store
- GIVEN the 0.5.1, 0.7.0 or 0.8.0 fixture, unreconciled
- WHEN it is indexed and asked every field of SEARCH §8
- THEN every query answers, its task of no priority and no category, and pimdir.db is unwritten (tests/store_version.rs `a_store_released_by_*_is_indexed_and_searched`; 0.5.1 and 0.7.0 failed before: "no such table: store.file_summary")

### Requirement: A card's properties and categories are recorded at extraction
The extraction of a card SHALL record whether it holds a `TEL`, an `ADR`, a `BDAY` and a `PHOTO` (`PimdirExtract::tel`, `adr`, `bday`, `photo`, written to `object.tel`, `adr`, `bday`, `photo` by `insert_object`) and its `CATEGORIES` values, folded as Annex A.2 folds `fn`, each once (`PimdirExtract::categories`, `insert_object_category`); every other kind records none. `has:phone`, `has:address`, `has:birthday` and `has:photo` SHALL parse to `PimdirQueryPredicate::HasProperty` with `PimdirQueryProperty` and compile to `holding`; `category:` SHALL match a task on `store.task_summary` as before or a card through `categorised`.

#### Scenario: The card vectors
- tests/search.rs, the cases over item 25 (card-held.vcf) of pimdir vectors/search/queries.json, every one refused (`invalid-value`) or missing the card before this; an index of the earlier shape is rebuilt (`an_index_of_another_shape_is_rebuilt_never_queried`).

### Requirement: An agenda reads a range's occurrences in order
`PimdirIndex::occurrences_between(start, end)` SHALL answer every occurrence overlapping `[start, end)`, RFC 3339 instants in UTC, one `PimdirSearchOccurrence { collection, seq, start, end }` each, ordered by start, then `seq`, then collection (`occurrences_between`, SEARCH §7), within the index's horizon. An owner keeping an index calls `refresh` after each write batch it commits (SEARCH §4); the store never calls it, the index being optional.

#### Scenario: The occurrence windows
- tests/search.rs, every window of `occurrences` in pimdir vectors/search/queries.json: a weekly series as three rows in time order, an occurrence running into a range, a zero-length task at a range's start, the same at its end left out.

### Requirement: The index refreshes without I/O
The crate SHALL offer the index's refresh as a coroutine, `search::refresh::PimdirIndexRefresh::new(now, batch)`, folding one batch of the feed per run (SEARCH §4) and yielding typed reads and typed writes, never SQL, so an owner holding its own SQLite indexes without the crate opening a file. Its reads are `WantsIndexState` (`index_meta` with the store's change cursor and whether a collection moved), `WantsReexpand` (`items_to_reexpand`), `WantsFeed { since, limit }`, `WantsUnread` (`load_unread_bodies`), `WantsHeld` (the index's placement rows and object hashes among those asked), `WantsBodies { hashes }` (at most `BODIES_MAX`), `WantsSummaries`, `WantsMessages` (a message id's row, children, presence, held body and `In-Reply-To`) and `WantsGone`; its writes are `Writes(Vec<PimdirIndexOp>)`. A run is one transaction: a consumer answers every yield of it inside one write transaction, applies each `Writes` in order as it comes, every read after it seeing it, and commits when the run completes with `PimdirIndexRefreshOutput`: the cursor it recorded and whether the feed was exhausted, so a caller runs again or stops within its deadline. `PimdirIndex::refresh` SHALL be that coroutine answered through the canonical statements, so the reference implementation and an app run one fold.

Each `PimdirIndexOp` SHALL name SEARCH §3's rows, so a consumer of another shape maps it to its statements: `PutObject` (the `object` row, its card categories, its text and its chunks) and `DropObject`, `PutPlacement` and `DropPlacement` (its summary text, flags and occurrences with it), `PutSummaryText` and `DropSummaryText`, `ReplaceFlags`, `ReplaceOccurrences`, `PutMessage` and `DropMessage`, `PutThread` and `DropThread` (when no message points to it), `SetCursor`, `SetHorizon` and `SetTzdbVersion`. A relink's ops SHALL write the thread rows before the message rows pointing to them, and drop the stale thread rows last.

`WantsFeed` SHALL be answered in the crate's own types, never as the canonical statements' rows (`PimdirIndexFeedRow`: collection, seq, link id, object hash, flag set, deleted, retained, kind, account, stamp, and `PimdirIndexSummary`, the summary row with its lists and its address rows, on every live row holding no body and on every file), so a consumer of another shape maps its own rows into them. A purge or a collection rename SHALL be its own yield, `WantsGone`, asked once by the run exhausting the feed when the store's purge count or a collection's stamp above `store_reconciled` moved: the feed pages what the store holds by stamp, and a purged item, a collected object or a renamed collection's old id leaves no row and draws no stamp.

#### Scenario: An owner in another language
- GIVEN the search vectors' fixture store
- WHEN a consumer holding its index in plain maps answers every yield from the store and from its maps, in batches of four, then again after a collection rename and a horizon rolled a week later
- THEN its placements, objects, card categories, chunks, the bodies and placements holding text, flags, occurrences, messages, threads and cursor are the index.db `PimdirIndex` builds, row for row (tests/search.rs `memory::a_consumer_of_its_own_shape_folds_the_canonical_index`)

### Requirement: Search compiles without the client
The query language, the folds, extraction, chunks, occurrences, tags, thread derivation and the refresh coroutine SHALL build without the `client` feature, under src/search/, using neither std I/O nor rusqlite; occurrence and thread derivation stand apart from the statements writing them (src/client/search/refresh.rs).

#### Scenario: A build without rusqlite
- GIVEN the crate built with `--no-default-features --features search`
- WHEN a caller parses a query, extracts a message and runs a refresh answering it by hand
- THEN all three succeed (tests/search_pure.rs), and `cargo tree --no-default-features --features search -e normal` names no SQLite

### Requirement: The index folds text before the tokenizer
The index SHALL store and match text folded by SEARCH §5's rule (`search::fold::text`) before any tokenizer reads it, the refresh's ops carrying it folded already, and SHALL register no SQL function: `category:` and the `name` order fold both sides in Rust (`search::fold::value`, STORAGE Annex A.2), the `name` order sorting the hits' titles after reading them.

#### Scenario: A diacritic
- GIVEN a message whose subject is "Réunion"
- WHEN the query `reunion` runs
- THEN the message is a hit, whatever the shape's tokenizer (the search vector `αθηνα` over item 8, "Budget Αθήνα", which `remove_diacritics 2` would miss)
