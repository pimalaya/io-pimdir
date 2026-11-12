---
cairn: spec
capability: search
status: current
---

# Search

The reference implementation of pimdir SEARCH.md (`search-draft-10`, an indexer and a query client of the canonical shape), experimental: its API and index build may change in a minor release: index.db beside the store, its refresh from the change feed, extraction and the query language. It ships behind the `search` feature, which `cli` enables and the default leaves out, so a consumer that does not search carries none of it. `search` alone is I/O-free (src/search/: the query language, the folds, extraction, chunks, occurrences, tags, threads and the refresh coroutine); `search` with `client` adds `PimdirIndex`, index.db over rusqlite (src/client/search.rs). Milestones M1 to M4 of pimdir's search-milestones: every kind's extraction, every field, occurrences within the horizon, threads, relative dates, tagging and the thread verb; and search-draft-08's bounds, versions, retries and prune.

### Requirement: The index is derived and rebuilt, never migrated
`PimdirIndex::open` SHALL open index.db as the main database with pimdir.db attached read-only as `store`, and SHALL read what it holds before writing anything (SEARCH §2, `PimdirIndexHeld` in src/client/search.rs). It SHALL refuse as `PimdirError::IndexVersion` (`index-version`, naming the index's version and shape) an index.db holding a schema and no `index_meta`, one whose `shape` is not `canonical`, and one whose `format_version` lies above `search-draft-10` or names no draft, never rebuilding, writing nor deleting it: the journal mode is set only once the index is found its own or built. It SHALL rebuild one of its shape whose `format_version` lies below its own or which lacks the column, one at another schema version, tokenizer or schema text (every table, index and trigger migrations/search/0001_init.sql creates, compared by its `sqlite_schema` text with `--` comments removed and whitespace collapsed, the shadow tables FTS5 creates and any other table left out), one built from another store or another copy of it (`index_meta.store_id` and `store_incarnation` against `load_change_cursor`'s), and one whose `store_change` lies above the store's last stamp. `refresh` SHALL make the same check under index.lock and rebuild an index a store was restored under. Versions are ordered by their draft number below every release (`sql::search::FORMAT_DRAFT`). Stored chunk offsets are always this crate's extraction's, which a snippet re-extracts (SEARCH §6). `PimdirIndex::open_query` SHALL refuse an index of another shape or version as `IndexVersion` and one this build would otherwise rebuild as `PimdirError::IndexOutdated`. It SHALL refuse a SQLite without FTS5 or older than 3.43 (`PimdirError::Fts5`) rather than build a partial index.

#### Scenario: An index of another schema text
- GIVEN an index.db of this version whose `placement_by_link` index was dropped
- WHEN `open_query` opens it, THEN it answers `IndexOutdated`; WHEN `open` opens it, THEN it rebuilds it and the next `open_query` opens it (tests/search.rs `an_index_of_another_schema_text_is_rebuilt_never_queried`)

#### Scenario: Another shape, a later version, no index_meta
- GIVEN an index.db whose `shape` is `fts4`, whose `format_version` is `search-draft-11` or `search-v1`, or holding no `index_meta`
- WHEN `open` or `open_query` opens it, THEN each answers `IndexVersion` naming what it found, and index.db is byte for byte what it was, no WAL frame written (tests/search.rs `an_index_of_another_shape_or_a_later_version_is_refused_and_left_as_it_is`)

#### Scenario: An earlier version
- GIVEN an index.db at `search-draft-07` or `search-draft-09`, or lacking `format_version`
- WHEN `open_query` opens it, THEN it answers `IndexVersion`; WHEN `open` opens it, THEN it is rebuilt at `search-draft-10` (tests/search.rs `an_index_of_an_earlier_version_is_rebuilt_never_queried`)

#### Scenario: Another store, another copy, a cursor ahead
- GIVEN an index.db naming another `store_id` or `store_incarnation`, or a `store_change` above the store's last stamp, or a store whose incarnation `raise_incarnation` raised under an open index
- WHEN `open_query` opens it, THEN it answers `IndexOutdated`; WHEN `open` opens it or `refresh` folds into it, THEN it is rebuilt from the store as it stands (tests/search.rs `an_index_built_from_another_store_or_copy_is_rebuilt`)

### Requirement: The index statements are bound as re-signed
The index SHALL bind the search statements as search-draft-10 signs them (SEARCH §2, §3): `init_index_meta` writing the format version, the shape and the store's id and incarnation off the attached store, `load_index_meta` read by its twelve columns (`version`, `tokenizer`, `store_change`, `store_purges`, `horizon_start`, `horizon_end`, `tzdb_version`, `store_id`, `store_incarnation`, `format_version`, `shape`, `floating_zone`), `set_index_cursor` with `:store_change` and `:store_purges` alone, `items_to_reexpand` with `:floating_zone` bound to the zone the refresh states, `placements_gone` read by its three columns, the link id naming the thread to prune, `coverage` and `cut_before` with the `collections` slot, and `hit` with `:seq` and `:account`, read by its nine columns (`collection`, `link_id`, `flags`, `object_hash`, `sort_key`, `level`, `kind`, `kind_value`, `title`): every live placement of a hit in collection order, the first naming its kind (the `kind:` value, else the collection's media type for a calendar item holding no summary row) and its title. The crate keeps no text of its own for it. Every statement of SEARCH §3's table is run. The order by relevance is the crate's own statement over the rank `match_objects` and `match_summaries` answer, the whole hit set in one statement, bodies and bodiless placements' summaries ranked together (SEARCH §8.2).

#### Scenario: A roll over a recurring journal
- GIVEN an index holding a journal with a yearly rule, expanded under a horizon
- WHEN a refresh rolls the horizon, THEN `items_to_reexpand` names the journal, whose summary does not say whether it recurs, and its occurrences are those of the new window; a refresh under the same horizon expands none (tests/search.rs `a_rolled_horizon_re_expands_a_recurring_journal`, which fails on the draft-09 text)

#### Scenario: An empty collection deleted
- GIVEN an index at rest and an empty collection of its store
- WHEN the collection is deleted, THEN the store draws no stamp and counts one purge, and the next refresh reconciles its keys, removing nothing (tests/search.rs `an_empty_collection_s_delete_is_reconciled`)

### Requirement: A refresh folds the feed in bounded, resumable batches
`PimdirIndex::refresh` SHALL take index.lock without waiting, answering `busy` when another indexer holds it. It SHALL fold `load_changed_items` in transactions of a bounded number of rows, each one run of `PimdirIndexRefresh` answered through the canonical statements (src/client/search/refresh.rs), each full batch recording through `set_index_cursor` its last row's stamp with the purge count the last completed pass recorded, so a pass cut short resumes where it stopped. The first run of a pass SHALL first fold again the placements `missing` records (`PimdirIndexRefresh::missing`). The transaction exhausting the feed SHALL reconcile keys when the store's purge count moved from `index_meta.store_purges`, and record the cursor read before the pass as `store_change` with the purge count read beside it. A deadline SHALL be checked between batches. A handle opened by `open_query` SHALL refuse to refresh (`PimdirError::IndexQueryOnly`).

### Requirement: One-shot answers refresh first and say what is behind
`search` and `count` SHALL refresh up to the store's head within `--refresh-timeout` before answering (`0s` skips it), and SHALL count what stays unfolded as `coverage.local.unindexed`; every answer SHALL say in `coverage.local.behind` (`behind` in JSON) what the canonical `behind` answers: the change stamps the store drew above the index cursor, store-wide. A session that keeps the index open follows the feed instead.

### Requirement: Mail is extracted through mime-rs, chunked as SEARCH §6 says
Mail extraction SHALL go through mime-rs (`parser`, `base64`, `encoding`): its MIME tree walked under `MIME_DEPTH_MAX` and `MIME_ENTITIES_MAX`, the parts the summaries' walk reads (a text part at depth 64 included, none after the 10,000th entity, the parse itself bounding no entity count so a multipart holding the 10,000th is still split), transfer encodings, charsets, RFC 2047 and 2231, `encrypted` and `unparseable` statuses, the attachment field from the crate's own stand-in walk. A header field the extraction reads (`Subject`, the address fields, `In-Reply-To` and `References`) SHALL be read from its first occurrence in document order, its name matched as STORAGE Annex A.1 matches one, whitespace before the colon included, so the index holds the subject and the people the summary row states (SEARCH §6; `extract::first`; tests/search_pure.rs `the_extraction_reads_mail_as_the_summary_does`, `the_extraction_and_the_summary_read_one_field`, the second on the standard's fixture mail-repeated-fields.eml). Until draft-10's second batch it read the last. A text part disposed with any type but `inline` is an attachment, as Annex A.1's mark reads it, and gives no body text; until then `attachment` alone was. A text part SHALL be transcoded from its `charset`, UTF-8 without one and windows-1252 under a US-ASCII label (as the WHATWG Encoding Standard reads it), no byte order mark sniffed and `format=flowed` text kept as written; a part of a `multipart/digest` stating no type is a `message/rfc822` and gives no text (RFC 2046 §5.1.5). Within an alternative the first child that is or holds a `text/plain` part SHALL be read, else the last; `text/html` SHALL be reduced to text by src/search/html.rs, the rules of mail-parser 0.11's `html_to_text` and its named character references, which mime-rs does not offer yet, but for one: a tag SHALL separate the text on its two sides, as whitespace does, unless it is the tag of an inline element, so `<td>Order number</td><td>12345</td>` reads `Order number 12345` and `H<sub>2</sub>O` stays one token. The inline elements (`INLINE`) are `a`, `abbr`, `acronym`, `b`, `bdi`, `bdo`, `big`, `cite`, `code`, `data`, `del`, `dfn`, `em`, `font`, `i`, `ins`, `kbd`, `mark`, `nobr`, `q`, `rp`, `rt`, `ruby`, `s`, `samp`, `small`, `span`, `strike`, `strong`, `sub`, `sup`, `time`, `tt`, `u`, `var` and `wbr`, compared by the tag's first word whatever its case and its attributes; every other tag separates, `br`, `img`, the form controls and a name HTML does not define included, and a comment separates nothing. A separator never opens the text nor a line, and a token ended by a tag takes the space owed before it by the same rule as one ended by whitespace (src/search/html.rs `a_tag_separates_unless_inline`; `tags_go_and_line_breaks_stay` read `HelloWorld` for two `div`s before). The reduction is the crate's own, SEARCH §6 stating none, and a body indexed before this keeps its text until its object is indexed again. A body whose first block holds no header field SHALL read as the text after its first line of blanks, and as `unparseable` when no such line ends that block. The `body` and `note` fields SHALL be cut into chunks by `search::chunk::chunks` on SEARCH §6's rule. Contacts and calendar items SHALL be extracted through vcard-rs and ical-rs on SEARCH §6's table, a `TEL` as written and as its digits, every event, task and journal of a resource, overrides included; an item falls back to its summary only while bodiless or unparseable, and a file always reads its summary.

### Requirement: Calendar time is expanded through ical-rs within a rolling horizon
Occurrences SHALL be expanded by ical-rs's `IcalRecurTimeline` with zones from `ical::tzdb::zones`, so the resource's own `VTIMEZONE` wins over the bundled database, within a horizon of one year back and two forward around the refresh's `now`, under the bounds of SEARCH §7 (below). A floating time, a date and a `TZID` neither names SHALL be read in the zone the refresh states (below). An item SHALL be re-expanded when its body changes, when the horizon has rolled by a day or more, when `ical::tzdb::version()` differs from the recorded one, which `set_tzdb_version` then records, and when the floating zone differs from the recorded one. A resource holding overrides and no master yields its overrides. `date:` and `when:` SHALL compare a bodied calendar item's occurrences, and coverage SHALL report `horizonLeft` when a range leaves the horizon with a calendar collection in scope.

### Requirement: A query is parsed whole or refused with SEARCH §8.4's object
`PimdirQuery::parse` SHALL accept exactly SEARCH §8.3's grammar within its limits (below) and refuse anything else with a `PimdirQueryError` carrying §8.4's code and span, never reading an unknown field as text. On `unknown-field` its `expected` SHALL be empty, `fields` carrying the list, and a name that lowercases to a field SHALL be named in the message. A statement failing on a query the grammar accepted SHALL answer `PimdirError::Unanswerable`, never a raw SQL error. The AST SHALL be public, so a consumer can compile its own filter into it.

### Requirement: Every answer carries its coverage
A search answer SHALL carry a `PimdirSearchCoverage`: the local axis (bodiless mail, contacts and calendar items in its collections, never files, counted by the canonical `coverage` on the store's `items_bodiless`, and the items not yet folded), the temporal axis (`Full`, or `Bounded` by the collections' coverage rows), the remote axis (`LocalOnly` unless a consumer merged a server answer), whether the range left the horizon, and for a query bounded by date the calendar items its collections hold whose occurrences a bound cut before the range's end (`cut`, `cut_before`, its end `NULL` for a range open there). The roll-up keeps the worst of each axis, and an answer with a cut is not complete. The CLI prints the cut where it prints the coverage (`cut` in JSON). `PimdirReader::search_coverage` SHALL fill the same type for the store's `LIKE` reads, which stay for consumers built without the feature.

### Requirement: Hits are addressable and typed
A hit SHALL be `(account, seq)` with its placements, its kind and title, what it matched (`body`, `summary`, or none when only structured predicates matched) and a snippet cut from its chunks. What it matched SHALL be probed for a page's hits alone (SEARCH §8.2): each positive text term against a body a placement holds (`match_object`), else against the summary of one indexed bodiless (`match_summary`). The CLI's `--json` output SHALL use camelCase keys and publish its JSON Schema (`pimdir-index`, `pimdir-search` for hits and threads alike, `pimdir-count`, `pimdir-show`, `pimdir-thread`, `pimdir-tag`), a refused query failing with `query-refused`, and `show SEQ --account ACCOUNT` SHALL accept a hit's address as is.

### Requirement: The vectors are the bar
tests/search.rs SHALL run every case of pimdir vectors/search, vendored under spec/vectors/search (queries.json at `v` 3, its 179 cases and its occurrence windows, and chunks.json), against a store built as an owner leaves it, indexed at the vectors' `now` in `UTC`, then again in each zone a case states for those cases, asked through `open_query` stating it, comparing the hits, the coverage, `horizon_left`, `temporal` and `cut` where a case states them. Every case SHALL run: none is skipped, `threads` cases included, and tests/pending_vectors.txt lists none.

### Requirement: Threads are relinked by the refresh and listed on request
The refresh SHALL relink the messages a batch touched (SEARCH §9): the parent from the held body's first `In-Reply-To` id, else its last `References` id, each field read from its first occurrence (SEARCH §6; src/search/thread.rs `a_parent_is_read_from_the_first_occurrence`), or from `mail_summary.in_reply_to` for a bodiless item; a `message` row for every parent named, present or not; a cycle broken by ignoring the edge that would close it; the thread id the present member with the lowest `(sort_key, link_id)`, its new row written and members repointed before an unused row goes (`delete_thread`). A message no placement holds any more prunes its thread first (below). `thread:` SHALL accept the thread id or any message id in it, so an id renamed by a join still resolves. `PimdirIndex::threads` (CLI `search --threads`) SHALL group hits by account and thread, in the order of each thread's first hit, `count`, `first` and `last` covering the thread's live members in that account; an item that is not mail stands alone.

### Requirement: Dates resolve against a clock in a zone
A query's dates SHALL resolve against `PimdirSearchClock`: `now`, the system clock unless one is given, and `zone`, an IANA name, UTC when `None`, as SEARCH §8.1 states. A year, a month, a day, `today` and `yesterday` SHALL run from the first instant of their first day in the zone to the first instant of the day after their last (`compile::bounds`, through jiff), so a day a transition moves lasts 23 or 25 hours and one whose midnight a transition skips starts after the gap; the temporal coverage's `since` and `until` and `cut_before` read the same bounds. A relative amount SHALL stay an instant: alone it spans from its instant to now, as a range side it is its instant, and a span reaching before year 0 is clamped to it. A zone the IANA database does not name SHALL be refused as `PimdirError::IndexZone` (`index-zone-unknown`) by `search`, `threads`, `count` and `tag`. `PimdirIndex::floating_zone` SHALL answer the zone the index records (`index_meta.floating_zone`), on an `open_query` handle too, the one a client should state. The CLI's `search`, `count` and `tag` SHALL take `--zone ZONE`, the index's floating zone by default.

#### Scenario: 1 January in Paris
- GIVEN an event from 00:30 to 01:00 in Paris on 1 January 2027, a floating one at 23:30 that day and an all-day one, indexed in `Europe/Paris`
- WHEN `date:2026-12-31`, `date:2027-01-01`, `date:2027`, and `date:today` at 23:30 and 00:30 local are asked stating `Europe/Paris`
- THEN 31 December holds none of them, 1 January and 2027 all three, and today turns over at local midnight (tests/search.rs `every_search_vector_answers`, the vectors stating a `zone`; a clock ignoring its zone fails seven of them)

#### Scenario: A day a transition moves
- WHEN `2026-10-25` and `2027-03-28` are read in `Europe/Paris`, and `2026-03-08` in `America/Havana`
- THEN they last 25 and 23 hours, and the Havana day starts at 01:00, its midnight skipped (src/client/search/compile.rs `a_day_a_transition_moves_is_23_or_25_hours`, `calendar_spans_start_at_the_zone_midnight`)

#### Scenario: The index's zone and an unknown one
- WHEN a query client reads the floating zone before and after an indexer states `Europe/Paris`, then counts stating `Mars/Olympus`
- THEN it reads `UTC`, then `Europe/Paris`, and the count is `index-zone-unknown` (tests/search.rs `a_query_reads_the_index_zone_and_refuses_an_unknown_one`; the CLI's in tests/search_cli.rs `a_query_reads_its_days_in_its_zone`)

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
`PimdirIndex::open_query` SHALL open index.db read-only with pimdir.db attached read-only, take no lock and write nothing, the journal mode included (SEARCH §3): it never creates, rebuilds nor refreshes the index, which the store's indexer (its owner, at sync time) keeps; the temporary tables a query fills (`temp.param`, `temp.term_match`) are its connection's own. An index.db absent or holding no schema SHALL be `PimdirError::IndexMissing` (`index-missing`); one of another shape or version, or holding no `index_meta`, `PimdirError::IndexVersion` (`index-version`), naming both; one this build's indexer would rebuild otherwise `PimdirError::IndexOutdated` (`index-outdated`), each saying to run the store's sync engine where it rebuilds. `PimdirIndex::open` stays the indexer's open, building and rebuilding.

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
The purge count alone SHALL reconcile keys, a rename counting in `store_meta.purges` (SEARCH §4, STORAGE §4.5): a refresh reconciling keys when `index_meta.store_purges` lags the store's, which a batch never moves, so a collection renamed while a pass was cut by its deadline after folding past the rename's stamp is reconciled by the next completed pass, its old id's placements dropped.

#### Scenario: A cut pass
- tests/search.rs `a_rename_a_cut_pass_went_past_is_reconciled_by_the_next_one` (failed before the change it was written for: nothing reconciled), now through the purge count alone; tests/search.rs `memory::a_consumer_of_its_own_shape_folds_the_canonical_index` compares the cursor and the purge count row for row

### Requirement: A search answers at the scale of a mailbox
A refresh SHALL read one item's summary and addresses by seek (`get_mail` and its siblings by `(collection, seq)`, `load_addresses`), never by a scan of its collection, and a compiled term's membership SHALL never be `NULL`, so a negated term is one probe per hit; a page reads its hits' titles for the `name` order alone. Over 50,000 summary-only messages in one collection (tests/search_scale.rs, release build) the index builds in about 5 s, and `is:unread` answers a page of 50 in about 120 ms and a count in about 25 ms.

#### Scenario: 50,000 messages
- GIVEN 50,000 summary-only messages in one collection, a third read
- WHEN the index is built, then `is:unread`, `not is:unread`, `is:flagged`, `date:`, `from:`, `subject:` are asked a page and a count
- THEN the build takes about 5 s (454 s before) and every page and count under 0.2 s (`is:unread` 25 s before)

### Requirement: A card's properties and categories are recorded at extraction
The extraction of a card SHALL record whether it holds a `TEL`, an `ADR`, a `BDAY` and a `PHOTO` (`PimdirExtract::tel`, `adr`, `bday`, `photo`, written to `object.tel`, `adr`, `bday`, `photo` by `insert_object`) and its `CATEGORIES` values, folded as Annex A.2 folds `fn`, each once (`PimdirExtract::categories`, `insert_object_category`); every other kind records none. `has:phone`, `has:address`, `has:birthday` and `has:photo` SHALL parse to `PimdirQueryPredicate::HasProperty` with `PimdirQueryProperty` and compile to `holding`; `category:` SHALL match a task on `store.task_summary` as before or a card through `categorised`.

#### Scenario: The card vectors
- tests/search.rs, the cases over item 25 (card-held.vcf) of pimdir vectors/search/queries.json, every one refused (`invalid-value`) or missing the card before this; an index of the earlier schema text is rebuilt (`an_index_of_another_schema_text_is_rebuilt_never_queried`).

### Requirement: An agenda reads a range's occurrences in order
`PimdirIndex::occurrences_between(start, end)` SHALL answer every occurrence overlapping `[start, end)`, RFC 3339 instants in UTC, one `PimdirSearchOccurrence { collection, seq, start, end }` each, ordered by start, then `seq`, then collection (`occurrences_between`, SEARCH §7), within the index's horizon. An owner keeping an index calls `refresh` after each write batch it commits (SEARCH §4); the store never calls it, the index being optional.

#### Scenario: The occurrence windows
- tests/search.rs, every window of `occurrences` in pimdir vectors/search/queries.json: a weekly series as three rows in time order, an occurrence running into a range, a zero-length task at a range's start, the same at its end left out.

### Requirement: The index refreshes without I/O
The crate SHALL offer the index's refresh as a coroutine, `search::refresh::PimdirIndexRefresh::new(now, batch)`, folding one batch of the feed per run (SEARCH §4) and yielding typed reads and typed writes, never SQL, so an owner holding its own SQLite indexes without the crate opening a file. Its public fields state the floating zone (`floating_zone`, `UTC` by default) and whether the run begins a pass and so first folds the placements recorded missing (`missing`, true by default). Its reads are `WantsIndexState` (`index_meta` with the store's change cursor), `WantsReexpand` (`items_to_reexpand`, with the horizons, the time zone database version and the floating zone), `WantsMissing` (`load_missing_items`), `WantsFeed { since, limit }`, `WantsHeld` (the index's placement rows and object hashes among those asked), `WantsBodies { hashes }` (at most `BODIES_MAX`), `WantsSummaries`, `WantsMessages` (a message id's row, children, presence, held body and `In-Reply-To`), `WantsThreadRows` (`thread_rows`, the rows a prune left) and `WantsGone`; its writes are `Writes(Vec<PimdirIndexOp>)`. A run is one transaction: a consumer answers every yield of it inside one write transaction, applies each `Writes` in order as it comes, every read after it seeing it, and commits when the run completes with `PimdirIndexRefreshOutput`: the cursor it recorded and whether the feed was exhausted, so a caller runs again or stops within its deadline. `PimdirIndex::refresh` SHALL be that coroutine answered through the canonical statements, so the reference implementation and an app run one fold.

Each `PimdirIndexOp` SHALL name SEARCH §3's rows, so a consumer of another shape maps it to its statements: `PutObject` (the `object` row, its card categories, its text and its chunks) and `DropObject`, `PutPlacement` and `DropPlacement` (its summary text, flags, occurrences, cut and missing record with it), `PutSummaryText` and `DropSummaryText`, `ReplaceFlags`, `ReplaceOccurrences` (with the item's `cut`, `set_cut`), `PutMissing` and `DropMissing` (`insert_missing`, `delete_missing`; a placement recorded is indexed bodiless, or is a calendar item whose indexed body an expansion could not read), `PutMessage`, `PruneThread` (`unparent_messages` then `prune_messages`), `PutThread` and `DropThread` (`delete_thread`, when no message points to it), `SetCursor`, `SetHorizon`, `SetTzdbVersion` and `SetFloatingZone`. A relink's ops SHALL write the thread rows before the message rows pointing to them, and drop the stale thread rows last.

`WantsFeed` and `WantsMissing` SHALL be answered in the crate's own types, never as the canonical statements' rows (`PimdirIndexFeedRow`: collection, seq, link id, object hash, flag set, deleted, retained, kind, account, stamp, and `PimdirIndexSummary`, the summary row with its lists and its address rows, on every live row holding no body and on every file), so a consumer of another shape maps its own rows into them. A purge or a collection rename SHALL be its own yield, `WantsGone`, asked once by the run exhausting the feed when the store's purge count moved, answered as `placements_gone` names them (`PimdirIndexGonePlacement`, with the link id) and `objects_gone`: the feed pages what the store holds by stamp, and a purged item, a collected object or a renamed collection's old id leaves no row and draws no stamp.

#### Scenario: An owner in another language
- GIVEN the search vectors' fixture store
- WHEN a consumer holding its index in plain maps answers every yield from the store and from its maps, in batches of four, then again after a collection rename and a horizon rolled a week later
- THEN its placements, objects, card categories, chunks, the bodies and placements holding text, flags, occurrences, cuts, missing records, messages, threads, cursor, horizon, time zone database version and floating zone are the index.db `PimdirIndex` builds, row for row (tests/search.rs `memory::a_consumer_of_its_own_shape_folds_the_canonical_index`)

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

### Requirement: A query is bounded
A query over 1,024 Unicode scalar values, 32 groups open at once or 128 terms SHALL be refused as `limit-exceeded` (`search::query::LENGTH_MAX`, `GROUPS_MAX`, `TERMS_MAX`; SEARCH §8.3, §8.4), in SEARCH §8.4's pass order: the length first whatever else the query holds, spanning from the first value past it to the end; the nesting in the parentheses pass beside an unbalanced parenthesis, the leftmost fault answering, spanning the `(` opening the 33rd group; the count when the 129th term is read, before its own checks, spanning that term. The descent recurses once per group alone, so the nesting bound, checked first, bounds its depth; a run of negations is read in a loop.

#### Scenario: At and one past each bound
- the eight limit cases of spec/vectors/search/queries.json (tests/search.rs `every_search_vector_answers`), and src/search/query.rs `a_query_is_bounded_in_the_stated_pass_order`

### Requirement: A query of many terms stays one shallow statement
The compiled query SHALL read each text term's matches once, through `match_objects` and, where the field exists in a summary, `match_summaries`, into the connection's `temp.term_match` under the term's number before the query runs, the query reading that table: a query of 128 text terms is answered on SQLite 3.54, which refuses a statement expanding as many full-text subqueries per reference as nested too deep (its cursor guard of 2026-10-04).

#### Scenario: 128 terms
- the vector case `reunion` 128 times (tests/search.rs `every_search_vector_answers`), which bundled SQLite 3.54.1 refused before ("VIEWs and/or subqueries nested too deep") and the system SQLite 3.50 answered

### Requirement: Occurrences are bounded and a cut is reported
An item SHALL write at most 10,000 occurrences (`search::occurrence::OCCURRENCES_MAX`), the first by start then end of those overlapping the horizon, and SHALL examine at most 1,000,000 periods of a rule (`PERIODS_MAX`), a period being `INTERVAL` times `FREQ` from the one holding `DTSTART`, a period before the horizon counting (SEARCH §7). A rule whose first unexamined period starts before the horizon's end, and which neither its `UNTIL` nor its `COUNT` ends before it, is stopped there, its occurrences from that instant on not written. An item either bound stops SHALL record its cut with its occurrences (`set_cut`): the earlier of its 10,001st occurrence's start and its first unexamined period's, `NULL` otherwise. A dated answer's coverage SHALL count the items cut before its range's end (`cut_before`). Departure: the walk is ical-rs's, which skips ahead over periods a `BYxxx` part rules out; the period bound is computed from the rule's `DTSTART`, `FREQ` and `INTERVAL` rather than counted, and the 10,001 first occurrences of each timeline are taken in identity order before the item's are ordered, so an override moving an instance from past them to before them is left out.

#### Scenario: An hourly series and a sparse one
- spec/vectors/search store items 27 (`FREQ=HOURLY`, 10,000 occurrences, cut at 2028-03-23T16:00:00Z) and 28 (`FREQ=SECONDLY` limited to 1 August at midnight, its `DTSTART` alone, cut at its millionth second), their two occurrence windows and the four cases stating `cut` (tests/search.rs `every_search_vector_answers`); src/search/occurrence.rs `one_item_writes_at_most_the_bound_and_is_cut_at_the_next`, `a_rule_is_examined_for_at_most_the_period_bound`

### Requirement: An index names its version and shape
The indexer SHALL write `search-draft-10` (`sql::search::FORMAT_VERSION`) and `canonical` (`SHAPE`) in `index_meta` through `init_index_meta`, refuse an index of another shape or a later version as `PimdirError::IndexVersion`, and rebuild an earlier one of its own shape (SEARCH §2, §3), as "The index is derived and rebuilt, never migrated" states.

### Requirement: Floating times are read in a recorded zone
The indexer SHALL state its floating zone, an IANA name, `UTC` by default (`PimdirIndexRefreshOptions::floating_zone`, `PimdirIndexRefresh::floating_zone`, `search::occurrence::FLOATING_ZONE`; the CLI's `--floating-zone`), read a floating time and a date in it, a wall time a transition repeats or skips taking the offset before it (`summary::calendar::iana`), and re-expand every live calendar item when it differs from `index_meta.floating_zone`, recording its own (`set_floating_zone`) in that pass (SEARCH §7). A zone the IANA database does not name SHALL be refused as `PimdirError::IndexZone` (`index-zone-unknown`) before the refresh reads anything.

#### Scenario: Paris, then UTC again
- GIVEN a floating event at 09:00 and a UTC one at 12:00 on 1 August 2026
- WHEN the index is refreshed in `UTC`, then twice in `Europe/Paris`, then in `Mars/Olympus`, then in `UTC`
- THEN the floating one starts at 09:00Z, then 07:00Z, both items re-expanded once and `Europe/Paris` recorded, nothing re-expanded the second time, the unknown zone refused, and both re-expanded back (tests/search.rs `another_floating_zone_re_expands_every_calendar_item`)

### Requirement: A body that cannot be read is tried again
A placement whose body the blob directory does not hold, or whose bytes fail its name, SHALL be indexed bodiless, its summary text written, and recorded in `missing` (`insert_missing`); the first run of every pass SHALL fold the recorded placements again (`load_missing_items`), whether or not the store stamped them again, and one indexed or holding no body leaves the record (`delete_missing`), one deleted taking it by cascade (SEARCH §4).

A re-expansion (the horizon rolled, the time zone database or the floating zone is another) that cannot read the body of an item the index holds a body for SHALL keep the occurrences and the cut the item has and record its placement in `missing` (`PutMissing`), where it replaced them by none before, leaving a single event out of every agenda until the next zone change. The fold of the placements `load_missing_items` answers SHALL expand every calendar row holding a body, whatever body the index holds for it: the object row being there already, a fold expanding only a placement whose body changed would forget the record without expanding (`Fold::plan`). A calendar row whose expansion read its body has its occurrences replaced and leaves the record; one whose expansion could not read it stays recorded, keeping its occurrences when the index holds that same body and losing those of another body otherwise; and a feed row of a calendar item the page did not expand neither records nor forgets it, since it may be recorded for an expansion it still owes. So a calendar placement stays in `missing` for as long as its body stays unread, and its body is asked again by the first run of every pass.

#### Scenario: A re-expansion over a body that cannot be read
- GIVEN an event with a floating start, indexed and expanded in `UTC`, its blob then removed
- WHEN the index is refreshed twice in `Europe/Paris`, the blob is put back, and it is refreshed again
- THEN the first two refreshes expand nothing, the occurrence stays at its `UTC` instant and the placement is recorded missing; the third folds no feed row, indexes no body, expands the item at its `Europe/Paris` instant and forgets the record (tests/search.rs `a_re_expansion_keeps_the_occurrences_of_an_unread_body`; the occurrence went with the first refresh before)

#### Scenario: A blob put back under its name
- GIVEN a message whose blob is missing when the index folds it
- WHEN the blob is put back under its name, the store stamping nothing, and the index refreshed
- THEN the first refresh records it missing and matches its subject alone; the second indexes its body, folding no feed row, and forgets the record (tests/search.rs `a_body_that_could_not_be_read_is_folded_again`)

### Requirement: A message that leaves prunes its thread
When a mail placement that held its link id last goes, by a deleted row or the key reconciliation, the refresh SHALL prune its thread in the same transaction (SEARCH §9): its rows no live mail placement holds name no parent (`unparent_messages`), those no row names go (`prune_messages`), and the rows left (`thread_rows`) are re-keyed with the touched messages, each connected set named by its present member with the lowest `(sort_key, link_id)` and a thread row no message points to dropped (`delete_thread`), each pruned thread among them once the relink is written: a thread whose every row the prune removed is no seed's, so the re-keying never meets it (`PimdirIndexOp::DropThread` for each pruned thread, after the relink's ops). The relink reparents only the messages a placement holds; the crate's own `DELETE_MESSAGE` and `DELETE_THREAD_UNUSED` are gone.

#### Scenario: A chain losing its middle, then its head
- GIVEN a phantom parent p, a replying to p, b to a, c to b and d to a
- WHEN b, then a, is deleted and the index refreshed
- THEN its `message` and `thread` rows are each time those an index rebuilt from the store as it stands holds, b kept as c's phantom parent, p gone with a (tests/search.rs `a_message_that_leaves_prunes_its_thread_as_a_rebuild_would`, `a_deleted_member_leaves_its_thread_consistent`)

#### Scenario: A message alone in its thread
- GIVEN two unrelated mails indexed
- WHEN one is deleted and the index refreshed
- THEN its thread row goes with its message row, the rows those a rebuild writes (tests/search.rs `a_lone_message_that_leaves_drops_its_thread_row`, which kept the row before the drop of the pruned threads; tests/erase.rs met it as the erased mail's key left in index.db)

### Requirement: The indexer erases what the store erased
`PimdirIndex::erase` SHALL run the index's part of STORAGE §11.5 after `PimdirStore::erase` (store.md, "An erase removes an item's bytes"): a refresh folding the whole feed in the floating zone `index_meta` records, so no zone change re-expands the calendar; `optimize_text` under index.lock, merging every segment of `object_text` and `summary_text`; then `PRAGMA main.wal_checkpoint(TRUNCATE)` until every frame is checkpointed (SEARCH §4, §12). Another indexer holding index.lock is `PimdirError::Indexing`, a query-only handle `PimdirError::IndexQueryOnly`.

#### Scenario: The erased mail's words
- tests/erase.rs `an_erased_mail_leaves_both_files`, failing without `optimize_text` (the subject's tokens left in a segment), without the checkpoint of index.db, and without the drop of a pruned thread (the key left in a `thread` row)

### Requirement: The streamed walk and the tree agree on the attachments
The crate reads a message's parts three ways: the whole-body derivation, the streamed walk (`PimdirAttachmentWalk`, the summaries') and mime-rs's tree (the extraction's). The walkers are not merged, the streamed one carrying bounds Annex A.1 does not have. tests/walk_agrees.rs SHALL hold them together: the streamed walk against the derivation on the mark and the invitation, and, under `search`, against mime-rs's tree on the parts that make the attachment mark, by section and media type, over the same generator, whole and cut anywhere (`tree::a_built_message_has_the_attachments_its_tree_has`).

They part on one case Annex A.1 does not state: the last child of a multipart never closed, ended by an outer delimiter or by the end of the message. The summaries read it as no part (a child no delimiter ends, summaries.md), mime-rs as one, so the extraction may index the text of a part the mark and the stand-ins leave unread. The property compares under the summaries' reading and `tree::a_child_no_delimiter_ends_is_a_part_of_the_tree_alone` pins the difference until the standard states one. mime-rs also reads `Content-Type`, `Content-Transfer-Encoding` and `Content-Disposition` from their last occurrence where Annex A.1 reads a field from its first, which no test of this crate reaches.
