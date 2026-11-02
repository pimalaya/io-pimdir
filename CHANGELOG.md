# Changelog

All notable changes to this project are documented in this file. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0/).

## [Unreleased]

### Added

- Added pimdir storage-draft-08, sync-draft-07 and search-draft-08, vendored at pimdir fbd25c9 with vectors/ and payloads/ (spec/REVISION). **Breaking.**

  A store an earlier draft or an earlier release of this crate wrote is refused by every profile, never reconciled nor migrated: move it aside and let its sync engine create a new one, every app built on this crate reinstalling its store.

- Added `PimdirRemoteSnapshot::narrowed` and `enumerated`, what a connector states of a round its listing a date filter narrowed (pimdir SYNC §4, §5). **Breaking:** a struct literal names them; the constructors (`round`, `page`, `delta`, `band_head`) state neither.

  A connector narrowing a round by any date filter sets `narrowed` on every page of that round and on no other, and on its last page hands `enumerated`, every handle the collection holds read by an enumeration no date filter narrowed, where its source offers one: IMAP narrowing by `SENTSINCE` hands `UID SEARCH ALL`, JMAP narrowing by `after` an `Email/query` filtered by the mailbox alone, Graph narrowing by `receivedDateTime` the folder's message ids, Gmail narrowing by `after:` the label's ids where it lists them. Such a round finds absent only a member dated before it opened, or one the enumeration lacks: an undated draft received before the filter's window and a message whose `Date` runs ahead of its arrival are no longer dropped.

- Added `PimdirLoadSelection::LastPage { narrowed, enumerated }`, the load of a round's last page: a store answers `PimdirLoaded::unstamped` from `list_unstamped_bindings` bound with `:narrowed`, `:enumerated` and the enumeration in the `handles` slot, and `All` no longer answers it. **Breaking** for a storage of its own. A narrowed round its last page opens is opened in a write of its own first (`OpenRound`), its opening instant being the store's.
- Added the empty-listing park (pimdir SYNC §5, §8): a round that named no member, none of its pages nor heads listing one in this run or an earlier one, does not close while any based binding of its source is absent, whatever their number, and a rekey whose listing names no member writes nothing, both reported as `parked`, unless the run carries `accept_absences`.
- Added `PreferLocal`'s keep of the body it pushes over (pimdir SYNC §5, §9): the member's body is recorded as the binding's divergence and kept as a retained item, reported `Overruled`, the base moved onto it at the observed revision, in a write committed before the local body is pushed gated on that revision; a push rejected or replayed finds it kept and pushes again, keeping nothing twice.
- Added `upgrade::PimdirUpgradeOptions` (`conflict`, `push_content`; `From<&PimdirSyncOptions>`) and `PimdirUpgrade::options`: an upgrade fetching the body of a binding's divergence settles it by the source's policy (pimdir SYNC §6), `PreferLocal` keeping it and moving the base onto it, `PreferRemote` adopting it and keeping the local edit, `Manual` recording it; a create's landing fetch with another body settles the same way.
- Added the fill's settle (pimdir SYNC §9): an item conflict recorded without the item's own body is settled by the collection's `prefer-` policy when the upgrade reading that body fills the item, the body not adopted kept over the filling binding's handle and reported `Overruled`, the summary of an adopted diverging body derived from its bytes.
- Added the mutable revive at `Meta` (pimdir STORAGE §11.1, SYNC §6): a bodiless placement of a card, a calendar resource or a file reviving a retained row revives it holding no body, the retained body kept as a retained item keyed over the reviving handle and reported `Overruled`, never pushed over the member's revision.
- Added the mail agreement against every row holding a key (pimdir STORAGE §9, §14): the write mints a mail member, an `Add`, a `Copy` or a `Move` apart from a live item other sources or none bind, a tombstone or a retained row whose subject, date or sender it does not agree with, a retained row on its size too, never joining, landing nor reviving it; a create staged under the key's provisional handle moves to the minted key's.

- Added `PimdirError::Version { found }` (`store-version`): a store whose `store_meta.format_version` is not `sql::FORMAT_VERSION` (`storage-draft-08`), or which lacks the column, refused by every profile naming both versions and writing nothing (pimdir STORAGE §4.2, §6).
- Added `PimdirError::Damaged { object }` (`store-corrupt`): a store naming this draft whose schema text is not the canonical one, or whose `user_version` is not 1, refused by every profile.
- Added `PimdirError::SyncVersion { found }` (`sync-version`): a store whose sync state another engine version wrote, refused by `PimdirSourceStore::load` and `write`; the engine's first write records `sync::SYNC_VERSION` (`sync-draft-07`).
- Added `sql::FORMAT_VERSION` and `sync::SYNC_VERSION`.
- Added `PimdirReader::store_meta`, the store's `store_meta` row (`PimdirStoreMeta`): its format version, its three schema versions, its id, incarnation, algorithm, derivation and sync version.
- Added `PimdirStore::raise_incarnation(replaced)`, what a restore runs on its copy (pimdir STORAGE §4.5, §8).
- Added `PimdirAddressCursor` and `PimdirAddressPlacement::cursor`.
- Added `store info`'s store id, incarnation, format version, read and produce versions, derivation and sync version, in text and in `pimdir-store-info` (`storeId`, `incarnation`, `formatVersion`, `readVersion`, `produceVersion`, `derivation`, `syncVersion`).
- Added quarantine/ beside objects/ (pimdir STORAGE §3, §7): a body failing its name, or a link where a body or its shard belongs, is moved there by the owner under its name, a period and a suffix unique to the move, holding the staging lock exclusively and serialised against its own writers, never unlinked; nothing reads it as bodies, the collector never walks it, and only the user empties it (`PimdirStore::empty_quarantine`, below). `PimdirStore::quarantine(hash)` makes the move; the drain, the verification before a push, the upgrade and the dedup lookup make it when they find such a body; `PimdirBlobs::quarantined` lists the files.
- Added `PimdirHash::is_name` and `PimdirHash::LEN`: a name is 52 characters of `a-z2-7`, checked before any path is built from it (pimdir STORAGE §5).
- Added `PimdirBlobs::verified(hash)`: the body as an open file already read whole through that descriptor and checked against its name, then rewound, for whoever serves a range or copies a body out (pimdir STORAGE §7).
- Added `PimdirBlobs::scan`, answering `PimdirBlobScan { files, strays }`, and `PimdirGcReport::strays` (`PimdirBlobStray`, `PimdirBlobStrayKind`): the entries of objects/ that are no body, left in place by the collector and reported; `pimdir check` lists them as `strays` and the files of quarantine/ as `quarantined`, `pimdir gc` counts the strays.
- Added the upgrade's verification of the bodies it holds (pimdir STORAGE §7, SYNC §6): a `Full` upgrade yields `PimdirYield::WantsVerify` with the bodies its requested placements hold at `Full` as their source last agreed on them, and fetches again those the storage leaves out. A runner of its own answers this yield already for a sync. **Breaking** for a runner answering an upgrade's yields in a fixed order.
- Added rustix 1 (`fs`, `process`, `std`) as a direct dependency on Unix under `client`, for the link-free opens of a body and of a lock file and the owner check of a store's modes (decision 9 of the-crate-follows-storage-draft-08).
- Added the header parses of pimdir STORAGE Annex A.1 at revision 3 to `summary::mail`, each public for a connector reading a raw header field: `message_id` (a `Message-ID`, comments skipped, bracketless only without whitespace, `<`, `>` or `"` and with one `@`) and `display_name` (comments as spaces, each quoted-string its content), beside `message_ids`, `decode`, `addresses` and `instant`; `charset_decode` is now public, answering `None` for a charset no WHATWG label names; `MIME_ENTITIES_MAX` (10,000) beside `MIME_DEPTH_MAX`.
- Added jiff 0.2 (`alloc`, `tzdb-bundle-always`, no default features) as a direct dependency of the no_std core, so a `TZID` its resource defines no `VTIMEZONE` for resolves through the IANA database (decision 6 of the-crate-follows-storage-draft-08): the same crate and database version ical-rs's `tzdb` brings to the index. With `encoding_rs`, which `client` now enables, a stripped release probe deriving a summary and opening a store on the default features grew from 2.84 MB to 3.62 MB, 207 KB of it the bundled zone data; the `cli` binary carried both already.
- Added the re-derivation of a store an earlier Annex A revision wrote (pimdir STORAGE §6): an owner opening a store at revision 1 or 2 (`open`, `open_current`) re-derives every held message and task from its body, a keyset page of `list_held_mail` and `list_held_tasks` per transaction, restating the summary, addresses, lists, sort key and automatic references and stamping each item whose rows moved, then records revision 3 with the last page. Link ids are never re-derived.
- Added the payload schemas of pimdir STORAGE §15.3 and Annex B.2 to the queue codec: `codec::action_from_payload` reads every action and intent at `v: 1` exactly as payloads/ types it, an intent by its type in `intent`, and `PimdirActionError::UnknownField(member)` names a member its version does not define. A producer refuses a malformed payload at the enqueue as `PimdirError::Action` (`action-malformed`) naming the member, and the drain parks one, never applying it in part; vectors/payloads.json runs whole (tests/payloads.rs).
- Added `capability::is_name`, Annex B's grammar for a capability's name.
- Added the owner's backstop for an intent (pimdir STORAGE §15.6): the drain parks an Annex B.2 intent whose `source` declares something but not the intent's capability at its anchor, or the copy or occurrence it asks for, naming both (`Source smtp does not support mail.submit`); one whose source declares nothing is still skipped for its performer.
- Added `summary::strip_instructions(body)` and `PimdirProducer::enqueue_foreign_body(collection, body, action)`: a resource the user did not write as it stands (an invitation added to a calendar, an imported file) is queued without the `X-PIMDIR-` properties it carried (pimdir STORAGE §18).
- Added `summary::calendar::unschedule(body)`: `SCHEDULE-AGENT=NONE` on every `ORGANIZER` and `ATTENDEE` lacking `CLIENT` or `NONE` of every component naming an attendee, how a producer records the user's acceptance that nobody is notified (pimdir STORAGE Annex B.1).
- Added `PimdirStore::erase(collection, seq)` and `PimdirEraseReport`, and with `search` `PimdirIndex::erase()`: the erasure of pimdir STORAGE §11.5, the item's purge when it is retained or `delete_unbound_item` when it is live and no source binds it, the collector, a `TRUNCATE` checkpoint of pimdir.db until every frame is checkpointed, then the index refresh with its thread prune, `optimize_text` and the same checkpoint of index.db, so the item's key, subject, addresses and body words are in neither file nor its WAL. CLI `pimdir item erase SEQ`, with `pimdir-item-erase`.
- Added `PimdirStore::empty_quarantine` and `PimdirQuarantineReport`: every file and link of quarantine/ unlinked at the user's request, under the staging lock taken exclusively (pimdir STORAGE §7). CLI `pimdir quarantine empty`, with `pimdir-quarantine-empty`.
- Added `security::path_component(name)`, a name a server or a body gave made one path component (separators, `.`, `..`, NUL, control and bidirectional formatting characters neutralised), and `security::scrub(text)`, an error's text without the credentials it may carry (pimdir STORAGE §18).
- Added `client::modes`: `create_dir_all` and `create_dir` (`0700`), `options` (a file created `0600`) and `exposed`, a store's modes on Unix (pimdir STORAGE §18).
- Added `PimdirError::Exposed(path)` (`store-exposed`): a store whose directory or pimdir.db another user can read or write, or a link where a lock file belongs, refused by every profile before anything is opened (pimdir STORAGE §18). **Breaking** for a store others can read: restrict it (`chmod 700` its directory, `chmod 600` its pimdir.db).

- Added flag and list rows (the store calls no JSON function): a placement's flags and a binding's base flags are rows (`placement_flag`, `binding_base_flag`, with `items.flags_known` and `bindings.base_flags_known`), written after their row with `retain_flags` and `add_flags` and read back as one space-joined text, in no order; a message's `In-Reply-To` ids and a task's categories are rows too (`mail_in_reply_to`, `task_category`), read per page as the addresses are, the public summary types keeping their `Vec` fields; every list parameter is bound through `temp.param` (`create_param` on every connection, `clear_param` and `bind_param` before the statement); `queue.target_collection` carries a move's, a copy's or a superseding add's target, which a rename follows and the owner reads in place of the payload's. `PimdirAction::target_collection` and `retarget` are the pieces.
- Added `PimdirError::Flag(flag)` (`flag-invalid`): a flag the store cannot hold, empty or holding a space, a control or a non-ASCII character (pimdir STORAGE §13), is refused by a producer's enqueue, by an owner's write before anything is written, and parks a queued row another producer wrote, never trimmed. `codec::is_flag` and `PimdirFlags::invalid` answer it.
- Added `search::fold::text`, the text fold of pimdir SEARCH §5: lowercased as `str::to_lowercase`, decomposed, stripped of the marks of U+0300 to U+036F, composed again. Every indexed text and every query text term is folded before FTS5 reads it, under the tokenizer `unicode61 remove_diacritics 0`, so `αθηνα` matches `Αθήνα`, and a snippet is found by the same fold. The `search` feature depends on unicode-normalization 0.1 (no default features).
- Added `search::refresh::PimdirIndexRefresh`, the index's refresh as an I/O-free coroutine (pimdir SEARCH §4), one batch of the feed per run and one transaction per run, so an owner holding its own SQLite in another language keeps the canonical index without the crate opening a file. It yields typed reads, `WantsIndexState`, `WantsReexpand`, `WantsMissing`, `WantsFeed { since, limit }`, `WantsHeld`, `WantsBodies { hashes }` (at most `BODIES_MAX`), `WantsSummaries`, `WantsMessages`, `WantsThreadRows` and `WantsGone` (purges and renames), and typed writes, `Writes(Vec<PimdirIndexOp>)`, each op naming SEARCH §3's rows (`PutObject`, `DropObject`, `PutPlacement`, `DropPlacement`, `PutSummaryText`, `DropSummaryText`, `ReplaceFlags`, `ReplaceOccurrences`, `PutMissing`, `DropMissing`, `PutMessage`, `PruneThread`, `PutThread`, `DropThread`, `SetCursor`, `SetHorizon`, `SetTzdbVersion`, `SetFloatingZone`), its text folded already; the feed is answered in the crate's types (`PimdirIndexFeedRow`, `PimdirIndexSummary`). It completes with `PimdirIndexRefreshOutput`, the cursor it recorded and whether the feed was exhausted.
- Added `search::fold::value`, STORAGE Annex A.2's fold, which `category:` and the `name` order compare by, and `search::tag::flags`, a tag's values read as flags.
- Added `PimdirError::Refresh` (`index-refresh-broken`, feature `search`): the refresh coroutine resumed out of turn, a bug in `PimdirIndex::refresh`.

- Added a task's capture and waited-on person (`PimdirTaskSummary::created` and `contacts`, the `contact` address role, Annex A.4); a card's `has:phone`, `has:address`, `has:birthday`, `has:photo` and `category:` recorded at extraction (`PimdirQueryPredicate::HasProperty`); and `PimdirIndex::occurrences_between`, one row per occurrence in time order, for an agenda. An index built by an earlier version is rebuilt on open.
- Added items kept on the device (pimdir STORAGE §14.3): `PimdirStore::put_item(collection, body)` writes a card or a calendar item into a collection no source syncs, its key, summary, references and pins derived and written as a synced write does, refused on a synced collection (`PimdirError::Synced`) and for mail, files or an undeclared kind (`PimdirError::NotUnbound`); `PimdirStore::ensure_time_collection` declares the local time collection, `client::TIME_COLLECTION` (`\u{1}time`), where time entries kept on the device go.
- Added the organisation of tasks (pimdir STORAGE Annex A.4, §14.2, SEARCH §8): `PimdirTaskSummary` gains `priority`, `categories`, `parent` and `mid`, derived from `PRIORITY`, `CATEGORIES`, the parent `RELATED-TO` and a `mid:` `ATTACH` or `URL`, stored, read and printed by the CLI; a task refers to its project and to the mail it was made from (`PimdirReferenceRole::Parent`, `Source`), recorded at the write whichever end lands first; `status:`, `priority:` and `category:` search tasks, `category:` folding both sides in Rust (`search::fold::value`).
- Added the `search` feature (off by default, adding mail-parser 0.11 with `full_encoding`; `PimdirIndex` with `client` too): the first milestone of pimdir SEARCH.md, the full-text index over a store.
  - `client::search::PimdirIndex`: index.db beside the store, opened with pimdir.db attached read-only as `store`, rebuilt when its version, tokenizer or shape (its tables, indexes and triggers against migrations/search/0001_init.sql, SEARCH §2) is not this build's, refused as `PimdirError::Fts5` on a SQLite without FTS5 or older than 3.43 (`vendored`, the default, bundles one). `refresh` folds the change feed under an exclusive `index.lock` in transactions of `PimdirIndexRefreshOptions::batch` rows, each writing the cursor it reached, until the feed is exhausted or `deadline` passes; the last one reconciles keys when the store's purge count moved, a rename counting there. `PimdirIndexReport` says what it did, and `busy` when another indexer held the lock.
  - `search`, `count` and `item`: hits keyed `(account, seq)` with every live placement in the account (`PimdirSearchHit`), what a text term matched (`PimdirSearchMatched::Body` or `Summary`, `None` for field terms alone), a snippet re-read from the blob through the recorded chunks, the `date`, `name` and `relevance` orders (`PimdirSearchSort`), and the coverage of every answer; `item` reads one hit's item as extraction does (`PimdirSearchItem`).
  - `search::extract::PimdirExtract`: mail through mail-parser's MIME walk under `MIME_DEPTH_MAX`, `text/plain` preferred, `text/html` reduced to text, the attachments the stand-ins' walk names, `encrypted` and `unparseable` statuses; contacts, calendar items, files and bodiless placements from their summary and address rows until vCard and iCalendar extraction.
  - `search::query`: the SEARCH §8.3 grammar parsed to `PimdirQuery`, an AST a consumer may build by hand, and the SEARCH §8.4 refusal (`PimdirQueryError`, with its code, span in Unicode scalar values, what was expected, and for an unknown field the known fields alone, its message naming the lowercase field a capitalised one meant).
  - `search::chunk`: the chunk boundaries of SEARCH §6.
  - `sql::search`: the vendored migrations/search and queries/search, and this crate's own index statements.
- Added the second search milestone, cross-domain (pimdir SEARCH §6 to §8), to the `search` feature, which now also enables vcard-rs 0.5 and ical-rs 0.6 (with its bundled time zone database, `tzdb`).
  - Contacts and calendar items holding a body are extracted from it rather than from their summaries: `PimdirExtract::contact` reads `FN`, `N` and `NICKNAME` as the title, every `EMAIL` (also under `email`) and the `FN` as people, `ADR`, `ORG`, `TITLE`, `ROLE`, and `NOTE` and every `TEL`, written and as its digits alone, as the note; `PimdirExtract::calendar` reads every event, task and journal of the resource, overrides included, `SUMMARY`, `ORGANIZER` and `ATTENDEE` with their `CN` (also under their roles), `DESCRIPTION` as the body, `ATTACH` file names, `LOCATION` and `COMMENT`. `PimdirExtract::body` dispatches on the collection's kind; the note and body fields are chunked.
  - `person:` (the contacts whose title matches, and every item naming one of their addresses) and `linked:` and `linked:<role>:<id>` (either end of the store's references, the item named by seq or link id under every kind it is held under) are answered rather than refused.
  - Occurrences (SEARCH §7): every occurrence of a calendar item within the horizon, one year back and two forward around `PimdirIndexRefreshOptions::now` (the clock's by default), materialised by ical-rs's instant walk through the resource's own zones, else the bundled database, else floating, at most `OCCURRENCES_MAX` per item. An item is expanded when its body changes, and again through `items_to_reexpand` when the horizon rolls by a day or more or the database names another version, recorded with `set_tzdb_version`. `PimdirIndexReport` counts the items `expanded` and those `bounded` at the limit. `date:` and `when:` compare a calendar item's occurrences, a bodiless one's sort key.
  - `PimdirSearchCoverage::horizon_left`: whether a query's date range leaves the horizon with a calendar in scope; `is_complete()` is false when it does, and the CLI prints it as `horizonLeft`.
  - A statement a compiled query fails with is `PimdirError::Unanswerable`, never a raw SQL error.
- Added the third search milestone, threads and relative dates (pimdir SEARCH §8.1, §9), to the `search` feature.
  - Threads: a refresh fills the index's `message` and `thread` tables from each message's first `In-Reply-To` id, else its last `References` id (read through mail-parser from the body the index holds, else from the summary's `in_reply_to`), a row for every parent named whether the store holds it or not, the edge closing a cycle ignored, and the thread named after its present member with the lowest `(sort_key, link_id)`, renamed as threads join, the new row written and the members repointed before the old one goes; a dropped message leaves its thread recomputed. No joining by subject.
  - `thread:<id>` answers the live messages of the thread, named by its id or by any message id in it, so an id a join renamed still reaches it.
  - `PimdirIndex::threads`: the threaded listing, the hits collapsed to their thread in their account (`PimdirSearchThreadsOutput`, `PimdirSearchThread` with the thread id, its live message count in the account, its first and last date, and its hits), an item other than mail standing alone; paged by thread.
  - Relative dates: `today`, `yesterday` and `3d`, `2w`, `1y`, alone or as either side of a range, resolved against `PimdirSearchClock::now` (the clock's by default), calendar spans in UTC. `PimdirSearchOptions::clock` carries it, and `count` takes it. `PimdirQueryDate` has `Today`, `Yesterday` and a typed `PimdirQueryDateUnit`.
  - CLI: `search --threads` prints the threaded listing, with its JSON Schema `pimdir-search-threads`.
- Added tagging and the thread of an item, the fourth search milestone's part in this crate (pimdir SEARCH §9, §10).
  - `PimdirIndex::tag(query, add, remove, source, clock)` answers a `search::tag::PimdirSearchTagPlan` without writing: every hit of the query, each live placement with its flags and the flag set it would carry (`next`, `None` when its flags were never read), planned on top of the last `set-flags` the queue holds for it so tags staged in a row compose. Each placement names the source its action is staged as (`PimdirSearchTagSource`): `source` when it syncs the collection, else the only source that does; several and none asked is `Ambiguous`, an asked source not syncing it, or none at all, `Unsynced`, and neither is staged. `syncs` says whether that source carries the change to its server by its declared capabilities (Annex B.1): false for contacts and calendars, which have no flags there, and for mail on a source not declaring the flags changed; the tag is then kept in the store alone. A placement whose source declares no support for a capability the change needs names it in `missing` and is not staged, the others are; `stage` still runs every action it stages through the gate before appending any, so a declaration changed since the plan refuses the stage (`PimdirError::Unsupported`) with the queue untouched. `read`, `flagged`, `answered` and `draft` name `\Seen`, `\Flagged`, `\Answered` and `\Draft`; any other value is written as given. An empty change, a flag both added and removed, `unread`, `encrypted` and an empty value are `PimdirError::Tag` (`PimdirSearchTagError`).
  - `PimdirSearchTagPlan::stage(producer)` appends one `set-flags` action per placement it changes, every one through the capability gate before the first is appended, answering `PimdirSearchTagAction`s.
  - `PimdirIndex::thread(account, seq)`: the thread of one item as a `PimdirSearchThread` (`thread_of`, `thread_members`), its members oldest first; an item no thread holds is its own.
  - CLI `tag QUERY --add FLAG --remove FLAG [--dry-run]`: stages the plan as a producer, each placement as a source syncing its collection, so one query tags mail and events of different sources together, then drains when no other process owns the store, each placement reported with its `source` and the collection's `sources`, whether it `syncs`, as `planned`, `unchanged`, `unknownFlags`, `ambiguousSource`, `unsynced`, `unsupported` (with its `missing` capabilities), `applied`, `queued` or `refused` as the store shows it, with the JSON Schema `pimdir-tag`. CLI `thread SEQ --account ACCOUNT`, with `pimdir-thread`.
- Added `search::coverage::PimdirSearchCoverage`, outside the feature: what an answer could not see on three axes (local: bodiless items per kind, files never counted since they are indexed from their summary alone, and changes not indexed yet; temporal: `Full` or `Bounded` from the coverage rows; remote: `LocalOnly` or `Augmented { exhaustive }`), rolled up keeping the worst of each, with `is_complete()`. `PimdirReader::search_coverage` answers it for the `LIKE` reads (`search_mail`, `search_contacts`, `search_calendar`, `search_files`), which stay.
- Added the CLI verbs `index`, `search`, `count` and `show`, the `cli` feature now enabling `search` so the binary ships them, each with `--json` and a JSON Schema (`pimdir-index`, `pimdir-search`, `pimdir-count`, `pimdir-show`, and `pimdir-query-error` for the refusal `search` and `count` print before exiting with status 1). `search` and `count` refresh the index first within `--refresh-timeout` (default 2s).

- Added stand-ins recorded as a body enters (pimdir STORAGE §14.3): a write bringing a message's body into the store records, in its transaction, one file at `Meta` per part carrying `Content-Disposition: attachment` in the account's attachments collection (the file collection with the role `attachments`, else `<account>/\u0001attachments`, declared with that role) and the `attachment` reference from the message, whatever wrote the body (a sync, an upgrade, a drained `add`, a direct write). A body the account already held under its key (a move, a copy, a second label, the same body again) walks nothing, and bodies held before are never backfilled.
- Added `summary::mail::attachments`: `PimdirAttachmentWalk`, the walk of a message's parts fed a chunk at a time with bounded memory, and `attachments(body)`, answering each attachment's section, name, media type and decoded size as a `PimdirFileSummary`; `ATTACHMENTS_MAX`, `HEADER_MAX` and `LINE_HEAD`.
- Added the attachment mark and the invitation to the streamed walk (pimdir Annex A.1): `PimdirAttachmentWalk::finish_parts` and `parts(body)` answer `PimdirParts { attachments, attachment, invitation }`, equal to what `derive` reads off the whole body, a writer streaming a body to its blob now deriving its invitation too; `CALENDAR_LINE_MAX`.
- Added the canonical `held_mail_summary` statement (pimdir STORAGE §14).

- Added round heads (pimdir SYNC §4, §5). A run resuming an open round first asks for the round's head, so a message arriving during a long round shows at the next run rather than once the round has closed.
  - The connector side: `PimdirRemote::heads` (default none), `PimdirHeads`, `PimdirHead` (`Delta` from a checkpoint, `Band` from the instant the round opened), `PimdirListing::Head`, `PimdirEnumerate::head`, `PimdirEnumerated::HeadRejected` and `NoHead`, and `PimdirRemoteSnapshot::band_head`; `PimdirSync::heads` for a runner of its own, which `PimdirSourceStore::sync` sets from the remote.
  - A delta head stamps its members and records its checkpoint as the round's. A band head stamps nothing and records nothing, and a connector listing band heads lists the band again in every round's last page.
  - A rejected head position restarts the round over the run's scope.

- Added `summary::mail::MIME_DEPTH_MAX` (64), the deepest multipart level the walk of a message's parts opens.
- Added `PimdirStore::open_current(dir)`, an owner open that creates nothing (pimdir STORAGE §6): a store absent or holding no schema is refused as `PimdirError::NotCurrent(PimdirNotCurrent::{Absent, Version { found }})`, writing nothing, and every other store as `open` refuses it.
- Added an `add` superseding an item (pimdir STORAGE §15.3): `PimdirAction::Add::supersedes`, `codec::PimdirSupersedes { collection, seq }`. The drain applies the add and tombstones the superseded item in one transaction, parking the whole action when the add parks, and records the pair (`supersede_item`); the sync holds the superseded tombstone's delete while the add's create is pending (`PimdirLoaded::held`, `list_superseded_pending`), so a refused create keeps the item on its server; the gate reads the superseded item's `remove` beside the add; the reader's overlay hides it while the add is queued.
- Added the member a superseding create may be moved from (pimdir SYNC §3, §4): `placement::PimdirSuperseded { collection, handle, if_match }`, read by `PimdirStore`'s load from `list_superseded_bindings` into `PimdirLoaded::superseded` for a pending create keeping the superseded item's identity, and carried on its `PimdirChangeKind::Add` as `supersedes`, outside the change key. A connector able to move members (WebDAV `MOVE`) delivers the create by moving that member then writing the body, so a server keeping one `UID` per account never holds it twice.
- Added `capability::CALENDAR_COMPONENT_VTODO` (`calendar.component.vtodo`, pimdir STORAGE §15.6, Annex B), read only where a row states it: a source stating `none` on a calendar collection, or source-wide, refuses a VTODO there at the gate and at the drain, a source stating nothing refusing nothing; `PimdirReader::accepts_vtodo(collection)` answers it, and `capability::is_vtodo` and `capability::check_vtodo` are the pieces.
- Added `PimdirReader::replied(message_ids)` (pimdir `list_replied`): which of the given bare ids a live mail outside the `sent` and `drafts` roles replies to.
- Added `PimdirArgError::UnresumablePage` and `PimdirArgError::IncompleteListing`, the engine's refusal of a page breaking the seam (pimdir SYNC §4, §8), which an exhaustive match must now handle. **Breaking.**
- Added `PimdirProducer::stage_body(bytes)` and `PimdirProducer::blob_writer()`, which hash a body and write it inside the producer's staging window (pimdir STORAGE §8), answering the `PimdirObject` `enqueue` takes, so a collector can no longer unlink it before its row pins it.
- Added `client::blobs::PimdirBlobStage`, the write side of the blob directory, holding its handle's lock while it or a writer it opened lives, handed out by `PimdirStore::blob_stage()` to an owner streaming fetched bodies.
- Added `PimdirProducer::enqueue_add(collection, link_id, flags, object, supersedes)`, which refuses an `add` whose link id the collection holds live or has queued as `PimdirError::Duplicate` (`client::PimdirDuplicate::Live(seq)` or `Pending(row)`), appending nothing, and answers the row id otherwise.
- Added `intent::PimdirSubmit`, the `submit` intent payload (pimdir STORAGE Annex B.2) with `to_action` and a strict `from_payload`.
- Added `PimdirError::code()`, a stable kebab-case code per failure (`store-version`, `store-corrupt`, `store-uncreated`, `store-busy`, `sync-version`, `unsupported`, `item-duplicate`...) for a client to map to its own output.
- Added `client::PimdirSqlError` and `client::PimdirJsonError`, opaque errors giving their cause through `Error::source`.
- Added `PimdirProducer::enqueue_with_body(collection, body, |hash| action)`: the gate of pimdir STORAGE §15.6 and, for an `add`, the duplicate check run on the bytes in hand, then the body written and the row appended in one transaction, so a refused create or edit leaves no body in objects/.
- Added `PimdirError::Foreign` (`store-foreign`), a pimdir.db that is not a pimdir store; `PimdirError::Queued { collection }` (`collection-queued`), a collection deleted while it holds queued actions; and `PimdirError::Role { collection, role }` (`collection-role`), a role outside its collection's kind.
- Added `PimdirLinkId::from_hint` and `PimdirLinkId::RESERVED` (pimdir STORAGE §9): a stated `Message-ID` or `UID` beginning with `alt:`, `hash:`, `dup:`, `part:`, `file:` or `esc:` is filed under `esc:` and the hint; a connector strips the one leading `esc:` before naming it to its server.
- Added `collection::role_of_kind`, the role vocabulary of each kind (pimdir STORAGE §14).
- Added the Annex A derivation revision to the store (pimdir STORAGE §6): `store_meta.derivation`, written at creation and set with `set_derivation` by an owner's first derived write to its own revision, lower or higher than the store's.
- Added the mass-absence guard (pimdir SYNC §5, §8): a round's last page, or a rekey, finding more than half of at least 20 based bindings absent writes no drop and reports `PimdirSyncReport::parked` or `PimdirRekeyReport::parked` (`sync::PimdirParked`), the round staying open for the next run to list from its first page; `PimdirSyncOptions::accept_absences` and the `accept_absences` argument of `PimdirSourceStore::rekey` carry the caller's confirmation. **Breaking** for a struct literal of `PimdirSyncOptions` and a caller of `rekey`.
- Added `PimdirSyncEvent::Overruled { handle, link_id }`: a body a policy did not adopt (the shared body `prefer-incoming` replaced, the incoming one `prefer-existing` refused, the local edit `PreferRemote` dropped) is kept as a retained item under `dup:<link>#<handle>` and reported by the verb whose write kept it, on `PimdirUpgradeReport::events` and the new `mutate::PimdirMutateReport::events` for an upgrade and a mutation (pimdir SYNC §5, §9).
- Added `coroutine::PimdirWritten` and `PimdirArg::Written`: a write answers what the cross-source merge recorded beyond its ops (a divergence on the item, a body kept as a retained item), `PimdirSourceStore::write` returning it. **Breaking** for a caller reading `write`'s `()`.
- Added `PimdirYield::WantsVerify`, `PimdirArg::Verified` and `PimdirSourceStore::verify`: a sync has every body a chunk uploads read whole and checked against its name before pushing it, a missing or corrupt one withheld and counted in `PimdirSyncReport::unreadable` (pimdir STORAGE §7). **Breaking** for an exhaustive match on `PimdirYield`.
- Added `PimdirLinkId::stated`, the identity a stated key names on a server, its one leading `esc:` stripped (pimdir STORAGE §9).
- Added `if_match: Option<PimdirHash>` to `PimdirMutation::Edit` and `PimdirMutateError::Replaced`: an edit stating the body it was made over is refused, staging nothing, once another write replaced it (pimdir SYNC §7). **Breaking** for a struct literal of `Edit`.
- Added `PimdirIndex::open_query`, the query client's open of the search index (pimdir SEARCH §3): index.db read-only, no lock, never created, rebuilt nor refreshed, with `PimdirError::IndexMissing` (`index-missing`), `IndexOutdated` (`index-outdated`) and `IndexQueryOnly` (`index-query-only`, a refresh asked of it). `PimdirIndex::open` stays the indexer's. The `search` feature is experimental: its API and index build may change in a minor release.
- Added `PimdirSearchLocal::behind`: how many change stamps the store drew above the index cursor, in every search answer and the CLI's coverage (`behind`).
- Added the query limits of pimdir SEARCH §8.3: `search::query::LENGTH_MAX` (1,024 Unicode scalar values), `GROUPS_MAX` (32 groups open at once) and `TERMS_MAX` (128 terms), each refused as `PimdirQueryErrorCode::LimitExceeded` (`limit-exceeded`) in SEARCH §8.4's pass order: the length first whatever else the query holds, the nesting in the parentheses pass beside an unbalanced parenthesis (the leftmost answering), the count when the 129th term is reached, before its own checks. The descent recurses only per group, so a hostile query no longer reaches it deep.
- Added the occurrence bounds of pimdir SEARCH §7: `search::occurrence::PERIODS_MAX` (1,000,000 periods of a rule examined, counted from the period holding `DTSTART`, before the horizon included) beside `OCCURRENCES_MAX` (10,000), the first by start then end of those overlapping the horizon. An item a bound stops records its cut, the earlier of its 10,001st occurrence's start and its first unexamined period's (`set_cut`), and a query bounded by date counts, among its collections, the calendar items cut before its end in `PimdirSearchCoverage::cut` (`cut_before`), which the CLI prints in its coverage (`cut` in JSON).
- Added the index's version and shape (pimdir SEARCH §2): `sql::search::FORMAT_VERSION` (`search-draft-08`), `FORMAT_DRAFT` and `SHAPE` (`canonical`), and `PimdirError::IndexVersion { path, version, shape }` (`index-version`): `PimdirIndex::open` refuses an index.db holding a schema and no `index_meta`, one of another shape, or one at a later or unknown version, never rebuilding, writing nor deleting it (the journal mode is set only once the index is found its own), and rebuilds one of its shape at an earlier version or naming none; `open_query` refuses any index of another shape or version. An index built from another store or another copy of it (`store_id`, `store_incarnation`), or holding a cursor above the store's last stamp, is rebuilt by `open` and by a `refresh` finding the store restored under it, and refused by `open_query` as `index-outdated`.
- Added the floating zone (pimdir SEARCH §7): `PimdirIndexRefreshOptions::floating_zone` and `PimdirIndexRefresh::floating_zone`, an IANA name, `UTC` by default (`search::occurrence::FLOATING_ZONE`), in which floating times and dates are read; one other than `index_meta.floating_zone` re-expands every live calendar item and is recorded in that pass (`set_floating_zone`). An unknown zone is `PimdirError::IndexZone` (`index-zone-unknown`). The CLI's `index`, `search`, `count` and the other refreshing commands take `--floating-zone ZONE`.
- Added the `missing` retry (pimdir SEARCH §4): a body the blob directory does not hold, or failing its name, is indexed bodiless and recorded (`PimdirIndexOp::PutMissing`, `insert_missing`); the first run of every pass folds the recorded placements again (`PimdirIndexRefreshYield::WantsMissing`, `load_missing_items`; `PimdirIndexRefresh::missing`), whether or not the store stamped them, so a body fetched again under its name after a quarantine is indexed; one indexed or holding no body leaves the record (`DropMissing`, `delete_missing`).
- Added the thread prune (pimdir SEARCH §9): when a mail placement that held its link id last goes, by a deleted row or the key reconciliation, the refresh prunes its thread through `unparent_messages` and `prune_messages` (`PimdirIndexOp::PruneThread`) and re-keys what `thread_rows` answers (`PimdirIndexRefreshYield::WantsThreadRows`, `PimdirIndexRefreshArg::ThreadRows`, `PimdirIndexThreadRow`), dropping thread rows no message points to through `delete_thread`, so the rows are those a rebuild from the store as it stands writes.
- Added `PimdirProducer::enqueue_with_body_checked` and `PimdirEnqueued { id, partial }`: the one-step enqueue answering the partial supports the gate passed the action with, read with the body in hand, so a frontend no longer runs `check` a second time.
- Added `PimdirDrainReport::overruled` and `PimdirDrainOverruled { id, collection, link_id }`: the bodies a collection's conflict policy kept as retained items while the drain applied a row. `PimdirDrainReport` is no longer `Copy`. **Breaking.**
- Added `PimdirSyncReport::unanswered`, the pushes the connector reported no outcome for, and `PimdirSyncReport::rejected_creates`, the pending creates the remote rejected in the run (pimdir SYNC §5).
- Added `PimdirBlobReader`, which `PimdirBlobs::reader` answers: a stream failing with `ErrorKind::InvalidData` the read that reaches the length the file stated when opened, or the end of the file, when the bytes do not hash to the name, and a read of a byte past that length (pimdir STORAGE §7), with `metadata`, `len` (the stated length) and `is_empty`. **Breaking**: `reader` answered a `std::fs::File`.
- Added `PimdirError::Cutoff` (`cutoff-invalid`): `purge_retained_before` refuses a cutoff not written as the stamps are (`2026-06-01T00:00:00.000Z`), which a textual comparison would read past what was asked.
- Added `PimdirProducer::probe()`, which proves the store writable by a producer for an app's check without writing anything into it: it takes the staging lock shared without waiting and releases it, then takes the database's write lock and rolls it back; a collector holding the staging lock is `PimdirError::Busy` at once (pimdir STORAGE §8).
- Added the overruling edit (pimdir SYNC §7, §9): `PimdirMutation::Edit` gains `overrule: bool`, which an engine or a connector settling a conflict by a rule of its own sets (`false` for a person's resolution and a queued `update`). The diverging bodies it does not adopt, the binding's `conflict_object` then the item's, are kept as retained items under `dup:<link>#<handle>` and reported `Overruled`, and the write is refused while the conflict holds a body nobody has read, as `PimdirError::Unread` (`conflict-unread`). The mutation stages it as the new `PimdirWriteOp::Overrule { collection, handle }` before its upsert. **Breaking**: a struct literal of `Edit` names `overrule`, and an exhaustive match on `PimdirWriteOp` meets the new variant.
- Added `PimdirWriteOp::Settle { collection, handle }`, which a person's resolution, an `Edit` stating `resolves` that names the item's diverging body, stages before its upsert, so the write settles the item's conflict (pimdir SYNC §7). **Breaking:** an exhaustive match on `PimdirWriteOp` names it, and a storage of its own, or a bridge relaying write batches, has to apply it as the hub does.
- Added `resolves: Option<Vec<PimdirHash>>` to `PimdirMutation::Edit` and `PimdirMutateError::Unresolved { handle, met, resolves }` (pimdir SYNC §7): an `Edit` without `overrule` settles a conflict, the item's or its binding's, only as a person's resolution naming the `conflict_object` of each conflict it meets; one meeting a conflict its `resolves` does not name, one stating `resolves` and meeting none, and one over a diverging body nobody has read are refused, staging nothing. An ordinary edit (`None`) of a conflicted placement is refused where it settled the conflict silently. **Breaking:** a struct literal of `Edit` names it, and a conflict command or form states the bodies it showed (`list_item_conflicts`, `list_conflicts`).
- Added `PimdirMutateError::code()`, a stable kebab-case code per failure as `PimdirError::code` answers for the store: `handle-unknown`, `placement-unnamed`, `item-duplicate`, `body-replaced`, `conflict-unresolved`, `placement-undeliverable`, `coroutine-arg`.
- Added `PimdirLoaded::conflicted`, the loaded placements whose item is conflicted with the item's diverging body, which `PimdirStore`'s load answers from `PimdirHub::conflicted` and an `Edit` reads beside its binding's conflict (pimdir SYNC §7). **Breaking** for a struct literal of `PimdirLoaded`; a storage of its own answers it, or an `Edit` reads a placement it projects `Conflict` with no binding conflict as an item conflict nobody has read, never settled.
- Added `PimdirHub::conflicted`.

- Added the `charset-tables` feature (adding encoding_rs 0.8 alloc-only), which `client` enables (decision 5): `summary::mail::charset_decode` decodes every label of the WHATWG Encoding Standard through `Encoding::for_label_no_replacement`, `decode_without_bom_handling` so the label holds against a byte-order mark; gbk, big5, euc-jp, iso-2022-jp, shift_jis, euc-kr, the windows-125x and iso-8859-x rows and utf-16 decode through their tables, and an encoded word in a label the table names with no encoding behind it (iso-2022-kr among them), or in none, is kept as written (pimdir STORAGE Annex A.1). A build without the feature keeps an encoded word in a charset other than UTF-8 and windows-1252 as written.

### Changed

- Changed the attachment mark and the stand-ins to follow pimdir's rule (STORAGE Annex A.1, §14.3): a part with no `Content-Disposition` now counts when it is neither text nor multipart and carries no `Content-ID` (a lone PDF, a forwarded `message/rfc822`), and an unknown disposition counts as `attachment`, where only `Content-Disposition: attachment` did. The RFC 1847 protocol parts (every child of a `multipart/encrypted`, the signature of a `multipart/signed`) never count, a disposed signature included, nor does a multipart message's own disposition. A stand-in is recorded once, so a store keeps the marks it recorded before.
- Changed `load::PimdirLoadScope` to `PimdirLoadSelection`, `PimdirYield::WantsLoad`'s `scope` to `selection` and `PimdirSourceStore::load`'s second parameter likewise: STORAGE §2 calls `All`, `Handles` and `Links` a selection, a scope being the date window (decision 7 of the-crate-follows-storage-draft-08). **Breaking.**
- Changed `PimdirSourceStore::upgrade` to take a `PimdirUpgradeOptions` before the remote (`PimdirUpgradeOptions::default()` settles nothing, `Manual`). **Breaking.**
- Changed `PimdirParked::guard` to take whether the listing named a member, parking on any absence when it named none. **Breaking.**
- Changed `PreferLocal` meeting a member that carries no body to record the divergence as `Manual` does, conflicted at the observed revision with no `conflict_object`, pushing nothing, where it pushed the local body gated on that revision over a body nobody read (pimdir SYNC §5).

- Changed the queue codec to refuse what pimdir STORAGE §15.3 calls malformed, where it read it: an `add` without `object` or `flags`, a `set-flags` without `flags`, a flag set `null`, holding a flag twice or one with a space, a member a version does not define (it was ignored), a `seq` below 1, a body that is no object name, an empty `to`, `link_id` or `supersedes.collection`, a `set-performer` capability outside Annex B's grammar. `PimdirActionError::MissingField` now means a member missing alone, and a member of another type is `Invalid { field, reason }`, `supersedes`' members named `supersedes.collection` and `supersedes.seq`. **Breaking**: an action holding a flag set never read is no longer queued as `null`, and a restore (`pimdir item restore`) of an item whose flags were never read queues the empty set.
- Changed `intent::PimdirSubmit`, `PimdirCollectionCreate` and `PimdirInvitation` to require their `source` (`String`, where it was `Option<String>`), as the schemas do. **Breaking.**
- Changed `intent::PimdirSubmit` to carry no `object`: the message is the row's body, `queue.object_hash`, never a member of the payload, so `to_action(message)` takes it and a payload naming `object` is refused; `from` is non-empty, the null path gone with the schema. A `PimdirAction::Unknown` read from the queue carries the row's `object_hash`, and `codec::action_from_payload`, reading a payload alone, names no body for it. **Breaking.**
- Changed `PimdirInvitation::from_payload` to refuse a `partstat` outside `ACCEPTED`, `TENTATIVE` and `DECLINED`, and an item named by both or neither of `seq` and `link_id`, as `Invalid`; a cancel stating `partstat` is refused. **Breaking** for a consumer matching the error.
- Changed `PimdirStore::replace_action` to refuse, as `PimdirError::Action`, a change whose payload its schema refuses, the intent left queued.
- Changed the blob layout to one shard level, `objects/<hash[0:2]>/<hash>` (pimdir STORAGE §5), where it was two; `pimdir export` lays its dump the same way, its manifest at `format_version` 3. **Breaking**: every store is reinstalled with this release anyway.
- Changed `PimdirBlobs::path` to answer `io::Result<PathBuf>`, refusing a malformed name as `ErrorKind::InvalidInput`; `get`, `reader` and `verified` refuse one the same way, where a short name used to be read flat under objects/. **Breaking.**
- Changed a blob write to never replace a file (pimdir STORAGE §5): the temporary is linked to its name then unlinked (`std::fs::hard_link`), a name already held keeping what holds it, where a rename replaced it; the shard directory is flushed after, objects/ too when the shard is new. A filesystem without hard links now fails the write.
- Changed every read of a body to follow no symbolic link under objects/: the shard and the body are opened with `O_NOFOLLOW` through rustix on Unix, read by lstat first elsewhere; a link there reads as no body.
- Changed the collector to unlink only a regular file, read by lstat, whose well-formed name begins with its shard's two characters, the shard a well-formed directory and no link. `PimdirGcReport` is no longer `Copy`. **Breaking.**
- Changed the drain to park a queued `add` or `update` whose payload's `object` is not `queue.object_hash`, naming both (pimdir STORAGE §15.3), and a queued body failing its name to fail its row as `InvalidData` and move into quarantine/.
- Changed `PimdirSourceStore::lookup_objects` and `verify` to answer only bodies read whole and found to hash to their name, moving one failing into quarantine/; the attachment walk reads a body to its end before recording anything walked off it; `pimdir item export` and `pimdir export` check a body whole before serving or copying a byte.
- Changed `PimdirChangeCursor` to carry the store's `store_id` and `incarnation` (pimdir STORAGE §4.5); it is no longer `Copy`. **Breaking.**
- Changed `PimdirReader::address_placements` to answer a page, once per placement and role, after a `PimdirAddressCursor` and at most `limit` long (pimdir `list_address_placements`). **Breaking.**
- Changed `PimdirReader::count_mail_by_day`'s `shift` to the reader's offset from UTC in whole minutes, east positive (`Option<i64>`). **Breaking.**
- Changed `hash::PimdirHasher` to a BLAKE3 hasher of its own (`PimdirHasher::new`), and `PimdirBlobs::open` to take the store directory alone. **Breaking.**
- Changed every connection to set `synchronous = FULL` and `secure_delete = ON`, and `fullfsync = ON` on Apple platforms (pimdir STORAGE §4.1, §11.5).
- Changed the store's creation to run 0001_init.sql in a deferred transaction and set WAL after it, so the store frees pages incrementally (`auto_vacuum = INCREMENTAL`).
- Changed a public id to be shared within a kind alone (`seq_for_link_any` bound with `:collection`, pimdir STORAGE §9.1): a Message-ID spelling an event's UID draws its own.
- Changed the summaries to pimdir STORAGE Annex A revision 3 (`store_meta.derivation` 3). A message's header is read as Annex A.1 states it: comments skipped around a `Message-ID` and an `In-Reply-To` id, a `<` inside one ignored, a bracketless id taken only without whitespace, `<`, `>` or `"` and with one `@`, a bracketless `In-Reply-To` read word by word; a `Date` with its comments removed wherever they stand, five words (whitespace around a colon not counting), the seconds optional, a military letter but `J`, `mm` at most 59, a missing zone or a word after it unparseable, years before 1900 refused, the day existing in its month, `30 Feb` and `24:00` `NULL` where they were carried forward, a leap second read as 59; a malformed encoded word (a `Q` word's stray `=`, a `B` word's padding past two or a lone sextet) and one in a charset no WHATWG label names kept as written, where they were read as UTF-8; `iso-8859-1` and `us-ascii` read as windows-1252; only spaces and tabs trimmed; a comment in a display name read as a space, and a quoted-string unquoted wherever it stands in one. The `alt:` key of a message stating no `Message-ID`, and the key of one stating a bracketless or commented one, move with these columns; a store an earlier revision wrote keeps its link ids. **Breaking** for a writer comparing keys across versions.
- Changed a calendar sort key's zone resolution (pimdir STORAGE Annex A.3): a wall time a transition skips takes the offset before the gap, where it took the one after (`20260308T023000` in New York is `07:30:00Z`); a `TZID` with no `VTIMEZONE` resolves through the IANA database before it reads as floating; a `VTIMEZONE` observance's `RRULE` stepping by a fixed span is followed, and a yearly one stops at its `UNTIL` and `COUNT`, for at most 1,000,000 periods.
- Changed the walk of a message's parts to read at most 10,000 entities, the message's own counted, beside the 64 levels (pimdir STORAGE Annex A.1): a part after the 10,000th makes no attachment mark, invitation, stand-in nor extracted text; the derivation, the stand-ins' walk and the search extraction bound alike.

- Changed `PimdirAction::Add` to carry `supersedes: Option<PimdirSupersedes>`, which a struct literal now names (`None` for a plain add), and `PimdirLoaded` to carry `held`. `PimdirChangeKind::Add` now carries `supersedes: Option<PimdirSuperseded>` and `PimdirLoaded` `superseded`, both named by a struct literal.
- Changed `PimdirWriteOp::SetRoundCursor::cursor` to an `Option<PimdirCursor>`, `None` keeping the cursor a page landed, and the vendored `set_round_cursor` to coalesce it (pimdir STORAGE §10). Added `PimdirListing::Head` and two `PimdirEnumerated` variants, which an exhaustive match must now handle. **Breaking.**
- Changed piped `--json` output of `pimdir` to print one value per line, each schema of `json-schema` to carry an `$id` naming the contract version (`v0.8`), and added `pimdir-error`, the schema of the JSON failure object (pimalaya-cli `55b6693`).
- Changed `pimdir collection list --json` to print `{"collections": [...]}` instead of a bare array, so every payload is a JSON object (breaking).
- Changed `PimdirStore::cancel_action` to open the store through `open_current`: it creates no store.
- Changed the owner verbs of `pimdir` (`queue cancel`, `gc`, `item purge`, `check --fix`) to open the store as it stands, through `open_current`.
- Changed the package to leave out AGENTS.md, CLAUDE.md, deny.toml and the Nix files.
- Changed `PimdirBlobs` to be read-only: `writer` is gone, a body being written through `PimdirProducer::stage_body`, `PimdirProducer::blob_writer` or `PimdirStore::blob_stage`, so a reader's handle (`PimdirReader::blobs`) can no longer put a body in the tree outside any lock. **Breaking.**
- Changed `PimdirBlobWriter::commit` to take no hash and answer the `PimdirObject` of the bytes written, hashed as they passed. **Breaking.**
- Changed `PimdirError::Sql` and `PimdirError::Unanswerable` to wrap `PimdirSqlError`, and `PimdirError::Json` `PimdirJsonError`, so the public API names no rusqlite or serde_json type. **Breaking.**
- Changed `PimdirError`, `PimdirSyncEvent`, `PimdirEnumerated` and `PimdirQueryErrorCode` to `#[non_exhaustive]`: a match on one needs a wildcard arm. **Breaking.**
- Changed `PimdirAction::Update` to carry `if_match: Option<PimdirHash>`, the body the producer read and replaced (pimdir STORAGE §15.3): the drain parks the row, naming both bodies, when the item holds another. A struct literal now names it (`None` applies over whatever the item holds). **Breaking.**
- Changed `PimdirAction::Update` to carry `resolves: Option<Vec<PimdirHash>>`, the payload's `resolves` (pimdir STORAGE §15.3), a non-empty array of distinct object names read strictly: the drain parks an `update` meeting a conflict, the item's or that of the binding it runs through, naming it, unless `resolves` names each conflict's diverging body, and one stating `resolves` and meeting none; a resolution runs through a conflicted binding whose body it names, and an unbound item's `update` takes it on the same terms. A producer deciding a conflict states it, and no other update. **Breaking:** a struct literal names it (`None` for an ordinary edit).
- Changed a blob write finding its final name held (pimdir STORAGE §5): the file there is verified, kept when it hashes to its name; one failing it is moved into quarantine/ by an owner's write (`PimdirSourceStore::write`, `PimdirStore::put_item`, an owner's `PimdirBlobStage`), under the staging lock taken exclusively and the process's writer lock, and the body written again, while a producer's `stage_body`, `blob_writer` commit or enqueue with a body fails as `PimdirError::Io` (`InvalidData`) naming the name, enqueueing nothing. A write skipping a body already held reads it whole once.
- Changed the schema: `collections.kind` closed to the four kinds or `''`, `collections.role` open, `items.level` 1 or 2, and `collections_keep_queued`.
- Changed `bindings.shared_object` to pin the body it names, as `base_object` does (pimdir STORAGE §5): the refcount, `refcount_drift`, the recount and the collector count it, so the common ancestor a merge reads is never collected while a binding names it.
- Changed `PimdirBlobs::get` to check a body against its name (pimdir STORAGE §7): a file whose bytes do not hash to it answers `None`, as a missing one does; the drain retries the row and moves such a file into quarantine/ (above).
- Changed the drain to skip, rather than park, a row whose payload states a version `v` it does not know (pimdir STORAGE §15.2); one stating none still parks.
- Changed `PimdirStore::delete_collection` to refuse a collection holding a pending or parked queue row as `PimdirError::Queued`, where the cascade dropped the row with no receipt.
- Changed `PimdirStore::set_collection_role` to refuse a role outside the collection's kind itself, the schema no longer checking the column, and the reader to read a role it does not know, or another kind's, as `None`.
- Changed the key of a stated identity beginning with a reserved prefix to `esc:` and the identity, so a vCard `UID` stating `hash:` keeps its own public id and references.
- Changed `PimdirChangeKind::SetFlags` to `SetFlags { handle, add, remove }`, two flag sets a connector applies as deltas, derived against the set the source listed, else the base, so a flag the server changed after the listing survives the push; its change key folds `add` then `remove` (pimdir SYNC §4). **Breaking** for every connector.
- Changed `PimdirSourceStore::mutate` to answer a `PimdirMutateReport`, and `PimdirUpgradeReport` to carry `events`, so it is no longer `Copy`. **Breaking.**
- Changed the cross-source merge (pimdir SYNC §9): a binding's `shared_object` moves only with a body the item settled on, never on a body-less upsert nor one leaving the item conflicted; a body filling an item a pull emptied is adopted without measuring a divergence; a divergence one of whose bodies nobody has read is recorded as `manual` records one, whatever the policy, and the upgrade fetching the late body settles it by the collection's policy.
- Changed a sync to report an event only once the write recording it committed (pimdir SYNC §5): `PimdirSync::report` no longer carries the events of a write not yet answered. A pull a write records as a divergence on the item reports `Conflicted` in place of `ContentChanged`.
- Changed `PimdirArgError::UnresumablePage` to cover a delta page that is not the last too.
- Changed the producer to hold the staging lock for its staging window alone, from its first staged body until an enqueue pinned every one, so an open producer that stages nothing no longer keeps the collector out (pimdir STORAGE §8).
- Changed the journal mode to be set when the owner creates a store, never on an existing one, so a store switched to a rollback journal for a network filesystem stays on it (pimdir STORAGE §8); README says how.
- Changed a reader, a producer and `open_current` to refuse a database declaring tables at `user_version` 0 as `PimdirError::Foreign` (`store-foreign`), as the owner open does, rather than `store-uncreated` or `store-not-current`.
- Changed a rekey to drop every old handle before any upsert, a handle the new space reuses for another member included, so a gone member under a reused handle is counted by the mass-absence guard (pimdir SYNC §8).
- Changed a page or an upgrade landing a pending create under a mutable member listed or fetched with no body to leave the binding conflicted at that revision with no diverging body, its base naming none, reported `Conflicted` beside `Created` and pushing nothing, whatever the source's policy; an upgrade fetching another body than the staged one records it as the conflict's `conflict_object` (pimdir SYNC §6). The landing claimed the server held the staged body at that revision, which nothing had read, so an edit another client made before the listing was never fetched and a later local edit pushed over it.

### Removed

- Removed `PimdirBlobs::files`, replaced by `PimdirBlobs::scan`. **Breaking.**
- Removed the reconciliation of an earlier draft's store: the rebuild, the backfills and the earlier drafts' lists, with the statements storage-draft-08 deletes (`repair_*`, `backfill_*`). **Breaking.**
- Removed `PimdirError::Stale`: a store of another draft is refused as `PimdirError::Version`. **Breaking.**
- Removed `hash::PimdirHashAlgo` and its `Sha256_128`, `PimdirStore::open_with_hash`, and every `hash_algo()` method: a store names its bodies by BLAKE3 alone, and `sha2` is gone from the dependencies. **Breaking.**

- Removed `codec::flags_to_json`, `codec::flags_from_json`, `codec::ids_to_json` and `codec::ids_from_json`: no store column holds JSON any more; `codec::flags_from_column` reads a space-joined flag set. **Breaking.**

### Fixed

- Fixed the streamed walk (`PimdirAttachmentWalk`) reading a message cut between the `\r` and the `\n` of its last line otherwise than `derive` (pimdir Annex A.1): a closing delimiter ending `--b--\r` closed nothing, so the part under way was dropped with its attachment mark, its stand-ins and its invitation, where `derive` read the multipart as closed. A line holding only a `\r` before its CRLF ended a header block in the streamed walk alone, dropping the fields after it. A `\r` ending the message now ends its last line in both, and a header line keeps its own `\r`.
- Fixed a person's `Edit` leaving its item's cross-source conflict standing, or recording its own body as a new divergence (pimdir SYNC §7): the hub cleared it only for an edit of a body one source alone changed, so keeping the item's body, or any pick made through the source the item moved away from, left the item conflicted. The edit now settles it whatever body it keeps and whichever source it is made through, the item taking its body, the diverging one released, nothing kept.
- Fixed a reader listing what is queued (`pending_actions`, `pending_creates`, `count_pending_creates`) failing on a row stating a later payload version, which the drain skips and leaves pending for a later owner, so one such row failed every frontend's status read for good: the row is skipped there too (pimdir STORAGE §15.2).
- Fixed the `store-version` and `sync-version` messages advising to move the store aside without first letting the sync engine that wrote it push what it queued, which lives in that store alone.
- Fixed a boundary that another boundary begins with (`d1` and `d10`): a line delimits a multipart body only when it is `--`, the boundary, `--` when it closes, then spaces and tabs alone (RFC 2046 §5.1.1), where `--d10` closed the level of `d1` and the parts under it were misnumbered.
- Fixed a mail row holding a body with columns read without it (pimdir Annex A.1). A body entering the store now states, in the write, the mark and invitation its walk gives over the writer's; a placement linked to a body the account already holds (an upgrade linking from the store, a copy, a move, a second label) takes the body's size and a held row's mark and invitation, where it kept its listing's guess forever. A body whose held rows state no mark (rows a writer recorded without reading the body) is walked from its blob for the mark and the invitation of the new row alone, recording no stand-in and leaving the held rows as they are, where the new row kept its listing's guess; a body missing from the blob store keeps the writer's mark.
- Fixed the invitation of a quoted-printable or base64 calendar part holding octets that are no UTF-8: the transfer encoding is now undone on the octets, before they are read as text.

- Fixed a stack overflow deriving a message nested thousands of multipart levels deep (about 350 KB on a 1 MiB stack): the walk for the attachment mark and the invitation stops at `MIME_DEPTH_MAX`. Fixed the same for a calendar body nesting components deeply, whose tree now keeps the three levels the derivation reads.

- Fixed a page landing a pending create over a mutable arrival holding another body: the arrival's body became the base and the staged one was pushed over it, so two servers holding one `UID` under two bodies ended with one of them. It is now the both-changed case, the conflict policy deciding (pimdir SYNC §6).
- Fixed a round page with `last` false and no cursor closing the round: every binding in scope it did not list was dropped as deleted, and the hub pushed the removal to every other source. The sync and the rekey now refuse it with `PimdirArgError::UnresumablePage`, writing nothing of it (pimdir SYNC §4).
- Fixed a rekey answered by a delta taking it as the whole collection and dropping every old placement it did not list: it is refused with `PimdirArgError::IncompleteListing` (pimdir SYNC §8).
- Fixed the drain applying a queued `add` or `update` whose body the blob store lacks, which created an item at `Full` naming a missing body, with no summary, or repointed an item to it. It is now a failure of the environment, retried then parked past `MAX_ATTEMPTS` (pimdir STORAGE §15.2).
- Fixed a pimdir.db another program wrote having its tables dropped by the owner open: it is refused as `PimdirError::Foreign` and left as it was.
- Fixed two writers storing one body sharing its temporary file, so one could rename the other's half-written bytes under a valid hash: the name is now unique to each writer (pimdir STORAGE §5).
- Fixed a local edit being dropped when another source's body-less pull had emptied the shared item: the hub projected the editing source `Clean`, so the edit was never pushed. It is now recorded as the item's conflict (pimdir SYNC §9).
- Fixed a `PreferRemote` pull carrying no body over another source's edit letting the next run push that edit over the server's unread one: the write records the item's conflict and the upgrade fetches the server's body into it (pimdir SYNC §5, §9).
- Fixed a policy (`prefer-incoming`, `prefer-existing`, `PreferRemote`) discarding the body it did not adopt: it is kept as a retained item and reported `Overruled`.
- Fixed a second source's message reusing a held `Message-ID` joining the held item when its subject, date or sender differ: it is minted apart, by the page's naming and by the write, so one body never stands for two messages (pimdir SYNC §6).
- Fixed an add accepted with no handle (IMAP without UIDPLUS) dropping its provisional binding and reporting `Created` for a member nothing named: it stays a pending create until the page naming it lands it (pimdir SYNC §4).
- Fixed a pending create or an edit whose body is missing or fails its name being pushed (pimdir STORAGE §7).
- Fixed a delta page that is not the last landing its checkpoint, which claimed the changes the pages after it list (pimdir SYNC §4).
- Fixed a listing carrying the very body a local edit holds being recorded as a conflict: the push whose record was lost had landed, and the base adopts it (pimdir SYNC §5).
- Fixed a flag merge with no base taking the remote set and dropping the local flags: it is the union of both (pimdir SYNC §5).
- Fixed a queued `update`, `set-flags` or `remove` on an item no source binds (written with `put_item`) being skipped by every drain and left pending for ever: a `remove` retains it, an `update` or a `set-flags` in a collection no source syncs applies to the store, and anything else parks naming the item.
- Fixed the overlay folding a queued `update` whose `if_match` names a body the item will not hold, an edit the drain parks.
- Fixed a queued `update`, or an engine `Edit`, made before a conflict arose settling it on drain and discarding the diverging body: one meeting a conflict it does not resolve parks (pimdir SYNC §7, STORAGE §15.3).
- Fixed the overlay folding a queued `update` the drain parks over a conflict: an ordinary one folds only over no conflict, a resolution only over the conflicts it names (pimdir STORAGE §15.4).
- Fixed a write keeping a file failing its name under the final name, part of a body a crashed exclusive create and copy left or other bytes, in place of the bytes in hand: the next read set it aside and the body was gone (pimdir STORAGE §5).
- Fixed a mutable handle stating a new identity dropping the local edit of the old one: it is re-staged as a pending create (pimdir SYNC §5).
- Fixed a rekey re-staging an edit no member carried under its old handle, which the renumbering may have given another member, so the store refused the batch and every later rekey: it takes its provisional handle.
- Fixed a rekey matching two copies of one hint at one revision by key order rather than by the body the listing carried, which carried one copy onto the other's handle and was refused (pimdir SYNC §8).
- Fixed an upgrade counting a `Full` fetch answering no body as fetched and upgraded.
- Fixed a consumer reading a streamed body to exactly its length, as an upload of a known size does, never having it verified: `PimdirBlobReader` checks it as the bytes read reach the length (pimdir STORAGE §7).

### Security

- Created every directory of a store (its own, objects/, the shards, quarantine/) `0700` and every file in it (pimdir.db, index.db, the lock files, the blobs) `0600` on Unix, the databases created before SQLite opens them so their `-wal` and `-shm` take that mode, and the directory a store is created in restricted to its user; `pimdir export` writes its dump so (pimdir STORAGE §18).
- Opened every lock file (owner.lock, objects.lock, index.lock) without following a link, `O_NOFOLLOW` through rustix on Unix and `FILE_FLAG_OPEN_REPARSE_POINT` on Windows, a link there failing the open as `PimdirError::Exposed` (pimdir STORAGE §18).
- Opened every database as one from elsewhere, whoever created it: `PRAGMA trusted_schema = OFF` and `SQLITE_DBCONFIG_DEFENSIVE` (SQLite 3.26 or later) on every connection, under which the canonical schemas and statements run, so a crafted trigger or view calls no function with side effects (pimdir STORAGE §18).
- Scrubbed the error a parked queue row records (`PimdirStore::fail_action`, the drain) of a URL's userinfo, an `Authorization`, `Cookie` or `Set-Cookie` header, a `Bearer` token and the value of a key naming a password, a token, a secret or an API key (pimdir STORAGE §9.2, §18).

## [0.8.0] - 2026-10-10

### Added

- Added `PimdirReader::sum_mail` and `PimdirMailSum`: the messages, known size and unknown sizes of a `[since, until)` range under the chips and a `held` chip, a message filed in several collections counted once (pimdir draft-04, STORAGE §14.1).
- Added `PimdirStore::release_before` and `PimdirReleaseReport`: the bodies of a set of mail collections below a date go back to `Meta`, headers kept, those still needed kept (pimdir draft-04, STORAGE §11.4).
- Added references between items (pimdir draft-04, STORAGE §14.2): the `reference` module (`PimdirEndpoint`, `PimdirReference`, `PimdirReferenceRole`, `PimdirReferenceOrigin`), `PimdirStore::add_reference` (answering the reference standing, `None` for an end missing or under a writer-derived key) and `remove_reference`, `PimdirReader::references_from` and `references_to`; the `item_reference` table, its index and its trigger are added on open to an older store, whose reader reads none.
- Added files (pimdir draft-04, STORAGE §14.3, Annex A.7): the `application/octet-stream` kind (`summary::file::KIND`), `PimdirFileSummary`, `PimdirSummary::File`, `summary::file::part_key` (`None` for a message under a writer-derived key), `PimdirStore::put_file` for a file holding no body in a collection no source syncs, `PimdirReader::list_attachments` and `PimdirAttachment`, `PimdirError::NotFiles`, and file pages in `list_summaries`; the `file_summary` table and the trigger collecting unreferenced stand-ins are added on open to an older store.
- Added automatic references (pimdir draft-04, STORAGE §14.2): a write recording a summary runs the canonical rules, a mail's senders (`link_senders_of`) and invitation (`link_invitations_of`), a contact's mail (`link_mail_from`), a calendar item's invitations (`link_invitations_to`), origin `auto`, never touching a person's.
- Added `PimdirMailSummary::invitation` (Annex A.1): the `UID` of the first `text/calendar` part, derived with the body and kept over a write without it; the column and its index are added on open to an older store. Opening an older store backfills it from the held bodies.
- Added `PimdirStore::delete_unbound` and the `attachments` role for a file collection, an older store's `collections` rebuilt on open to take it; `PimdirReader::list_attachments_by_account` with `PimdirAccountAttachment` and its cursor, `get_mail_row`, `describe_endpoint` with `PimdirEndpointView`, `search_contacts`, `search_files`, `search_calendar` with `PimdirCalendarHit`, and `PimdirLinkId::is_derived` (pimdir draft-04, STORAGE §14).

### Changed

- Changed the reconciliation on open to compare every table, index and trigger with the canonical schema's own text, comments dropped and whitespace collapsed, rebuilding a table that differs and recreating an index or trigger that does, instead of per-change checks (pimdir draft-04, STORAGE §6); a store missing a core table is refused before anything is reconciled.
- Changed a listing over a file holding its body to keep the size the body gave (pimdir draft-04, Annex A.7).
- Changed `PimdirStore::delete_collection` to skip the store-wide refcount recompute when the collection held no object pointer.
- Changed `PimdirStore::collect_before` to keep a mail a person's reference names (pimdir draft-04, STORAGE §11.3).
- Changed `PimdirReader::count_mail`, `count_mail_by_day`, `count_unread` and `list_mail_page_filtered` to take a `since` floor on the sort key, seeked on the index, undated mail falling below it; `None` keeps the previous behaviour (pimdir draft-04, STORAGE §14.1). **Breaking.**
- Changed a listing or a `Meta` fetch over a mail placement holding its body to keep the size read from the body, as it keeps the attachment mark, a source's stated size being possibly an estimate (pimdir draft-04, Annex A.1).

## [0.7.0] - 2026-10-09

### Added

- Added the `collection-create` intent (pimdir draft-04, Annex B.2): `capability::COLLECTION_CREATE`, declared in every domain's list, the `intent` module's `PimdirCollectionCreate`, and `PimdirProducer::enqueue_collection_create`, which anchors it on its parent and names its performer.
- Added occurrence replies and cancels: an optional `recurrence_id` in `calendar-reply` and `calendar-cancel`, gated on `capability::CALENDAR_REPLY_OCCURRENCE` and `CALENDAR_CANCEL_OCCURRENCE`; `PimdirInvitation`, `PimdirIntentItem` and `PimdirPartstat` build and read both payloads.
- Added queue receipts (STORAGE §15.4): the drain records what each applied row became, the `seq` an `add` created included, and prunes them after seven days; `PimdirProducer::action_status` and `PimdirReader::action_status` follow a row by the id its enqueue answered; the `receipts` table is created on open in an older store; `pimdir queue status`.
- Added `PimdirActionError::Invalid`, a payload field breaking its intent's rule.
- Added `PimdirStore::acknowledge_action(id, seq)`: an intent its owner performed is removed with its pin released and its receipt recorded, so `action_status` answers `Applied` rather than `Unknown`; `replace_action` records the replaced intent's receipt too. `drop_action` stays the withdrawal, without a receipt.
- Added collection roles (pimdir draft-04, STORAGE §14): `PimdirStore::set_collection_role`, `PimdirCollection::role`, the `ROLE` column of `pimdir collection list`; `collections.role`, its index and its triggers are added on open to a store written by 0.6, and a reader of one not reconciled yet reads `None`.
- Added scoped, paged mail syncs (pimdir draft-04, SYNC §4, §5): `PimdirSyncOptions::scope` (`PimdirScope`, `[since, until)` on the summary `date`, mail only, else `PimdirError::Scope`), rounds answered in pages and landed page by page (`PimdirWriteOp::OpenRound`, `Stamp`, `SetRoundCursor`, `CloseRound`, `SetCoverage`), resumed from their cursor, restarted on `PimdirEnumerated::CursorRejected`, and widened by the band alone for a connector bound to no scope (`PimdirRemote::scope_bound`, `PimdirSync::scope_bound`); deletes inferred at the last page in scope only; out-of-scope creates waiting (`PimdirSyncReport::waiting`); `PimdirSync::report` and `PimdirSourceStore::prepare_sync` for a runner of its own.
- Added coverage: `PimdirCoverage`, `PimdirRound` and `PimdirCursor`; `PimdirLoaded::coverage`, `round` and `unstamped`; `PimdirCollection::coverage`, `PimdirReader::list_coverage`, and the `COVERED` column of `pimdir collection list`. The columns, `bindings.round` and their trigger are added on open to an older store.
- Added the mail reads: `PimdirReader::count_mail`, `count_mail_by_day`, `count_unread`, `list_mail_page_filtered`, `search_mail`, `PimdirMailFilter`, `PimdirMailCursor`, `PimdirMailEntry`, `PimdirDayCount`, and `reader::like_pattern`.
- Added `PimdirStore::collect_before`, the owner's manual collection of a mail collection below a date (STORAGE §11.3).
- Added `summary::mail::derive_meta` and `meta_attachment`: the attachment mark read without the body (Annex A.1), which a body-less listing or fetch never writes over a walked one.

### Changed

- Changed the remote seam: `PimdirRemote::enumerate` takes a `PimdirEnumerate` (`PimdirListing`, delta or round from a cursor, and the scope) and answers a `PimdirEnumerated`; `PimdirRemoteItem` carries its `PimdirRemoteMeta` (hint, summary, sort key, optional body), and `PimdirRemoteSnapshot` is a page (`last`, `cursor`, an optional `checkpoint`, constructors `round`, `page`, `delta`). `PimdirYield::WantsEnumerate` and `PimdirArg::Enumerate` follow.
- Changed the sync to name every listed member in the page that lists it, landing a pending create its hint holds, and a pull to take the member's meta and carried body, the level lowered to `Meta`.
- Changed the rekey to list every page before its one batch and name members from their meta, fetching nothing.
- Changed `PimdirSyncOptions` to `Clone` without `Copy`, the scope holding strings.
- Changed a write to refuse an unnamed upsert of an unbound handle (`PimdirError::Unnamed`), and the items a batch inserts to draw their public ids in batch order.
- Changed a level of 0, an earlier draft's, to load as `Meta` with no summary, which a `Meta` upgrade revisits; `PimdirMutateError::Probed` is `Unnamed`.
- Changed a band round to infer no delete of an undated member, a band being listed by a date filter that never returns one (pimdir SYNC §5): `PimdirWriteOp::OpenRound` and `PimdirRound` carry `band`, recorded in `sources.round_band` (added on open to an older store), and an open round resumes only as the kind of round it opened as.
- Changed `list_mail_page_filtered` and `search_mail` to walk the new `items_by_sort_global` index across collections rather than sorting every live row of the set (pimdir draft-04, STORAGE §9.3); the index is created on open in an older store.

### Removed

- Removed probes (pimdir draft-04): `PimdirLevel::Probed`, `PimdirReader::count_probes`, `PimdirRekey::FETCH_CHUNK`, the `probes` table (dropped on open) and the probe statements.

### Fixed

- Fixed a `Remove` on a pending create, or on the copy the hub offers, leaving a binding until a sync: it now withdraws it in the same write, the item retained or its delete pushed by the other sources (pimdir SYNC §7).
- Fixed a copy or move into a collection already holding its identity, staged under a minted key, never being landed by its arrival and pushed a second time.
- Fixed a move of a body-less member into a collection already holding its identity deleting it from the server: the minted create now gets its origin, and the source's remove its destination with no `link_id`, so it relocates (pimdir SYNC §3, §4). A tombstone's destination now names the pending create's handle.

## [0.6.0] - 2026-10-03

### Added

- Added capabilities (pimdir draft-03, STORAGE §15.6, Annex B): the `capability` module (the vocabulary, `PimdirSupport`, `PimdirCapability`, `required`, `required_by_content`, `check`), the `capabilities` and `performers` tables, created on open in a store written by 0.5, and `PimdirStore::declare`.
- Added the producer's gate: `PimdirProducer::enqueue` refuses an action a declared source does not support with `PimdirError::Unsupported`, reading a calendar write's resources for scheduling, occurrence changes and online meetings, and `PimdirProducer::check` returns the partial supports to show.
- Added performers: `PimdirAction::SetPerformer`, `PimdirProducer::performer`, `PimdirReader::performers`, `PimdirError::NoPerformer` and `PimdirError::Ambiguous`. A choice holds from the moment it is queued.
- Added `PimdirStore::replace_action`, acknowledging a performed intent with the store change it leaves, such as the copy of a sent message.
- Added `PimdirReader::capabilities` and `item_capabilities`, and `summary::calendar::scheduled`, `occurrences` and `online_meeting`.

### Changed

- Changed the drain to park an action a declared source does not support, naming the capability, and to apply `set-performer`.

## [0.5.1] - 2026-10-01

### Changed

- Turned `vendored` on by default, as in himalaya, so `cargo install io-pimdir --features cli` builds SQLite from source and needs none on the machine. Use `--no-default-features` to link the system SQLite; the Nix builds still do.

### Fixed

- Fixed a server edit made while an item was in a cross-source conflict being dropped silently: the binding is now marked conflicted with the new revision, whatever the source's policy (pimdir draft-02, SYNC §5).
- Fixed a `Remove` settling a conflict leaving a base that paired the old body with the new revision, so a later revival read the server as in sync (SYNC §7).

## [0.5.0] - 2026-09-07

### Added

- Added the sync engine: the five verbs (`PimdirOpen`, `PimdirUpgrade`, `PimdirMutate`, `PimdirSync`, `PimdirRekey`) as I/O-free coroutines in the core, and `PimdirSourceStore::run`, `sync`, `upgrade`, `mutate`, `rekey`, `open_collection` and `service` running them against a consumer's `PimdirRemote`.
- Added typed summaries (STORAGE Annex A): `summary::derive` yields the key, the kind's summary row with the people it names and the sort key; the rows land in the five summary tables and `item_address`, and every listing read joins them (`list_summaries`, `get_item`, `address_placements`).
- Added the probes table, the change feed (`change_cursor`, `items_changed_since`, `collections_changed_since`), the refcount floor and every canonical statement the store lacked, so the schema is the specification's verbatim.
- Added the conformance suites: vectors/sync/ runs against the real store through a scripted connector (pushes compared with their idempotency keys), vectors/summaries.json against the derivations.
- Added `PimdirSync::beside_other_sources`, which decides a delete the source may not push: held beside another source of the collection, reverted for a source alone; the std client answers it from `collection_sources`, the bindings and checkpoints of the collection (SYNC §5).
- Added the re-staging of a local edit whose member the server deleted: a `Superseded` drop of its handle and a pending create under its provisional handle, never a drop of the edit (SYNC §5).
- Added `PimdirLinkId::provisional`, the `U+0001` handle namespace every staged create sits under, and `PimdirMutateError::Undeliverable` for a copy or move of a placement with neither a body nor a based binding (SYNC §3, §7).
- Added the item-level conflict to the projection: every binding of a conflicted item projects `Conflict` with no revision, an `Edit` or a `Remove` settles it, and `list_item_conflicts` lists what waits (SYNC §3, §7; STORAGE §14.1).
- Added `PimdirSourceStore::delete_collection`, `list_pending_actions` (store-wide, append order), `PimdirProducer::MAX_ATTEMPTS` and `PimdirError::InFlight`, returned by a collect while a verb of this process runs.
- Added the landing of a pending create by its arrival: an upgrade whose fetch resolves the hint a pending create holds supersedes the provisional handle instead of minting a second copy (SYNC §6).
- Added a `Tombstone` placement's destination (`DESTINATION_FOR_LINK`), so a move's remove relocates through the reference store, and `LOAD_PROBES_BY_HANDLE` for a `Handles` load.
- Added `sql::MIGRATIONS` and the migration runner of STORAGE §6 on the owner open.
- Added `PimdirRekey::FETCH_CHUNK`: a rekey resolves the new spine in bounded `Meta` fetches.
- Added the per-store writer lock serialising the collector against the process's own writers.
- Added the `json-schema` subcommand listing the JSON Schema of every command's `--json` output, and hidden plural aliases (`completions`, `manuals`, `collections`, `items`, `queues`, `stores`) on the CLI verbs.
- Added the release workflows and the nix derivation behind them, so a tag ships a `pimdir` binary per platform with its manual pages, completion scripts and JSON Schemas, and publishes the crate. The released binaries carry `cli` and `vendored`, each holding its own SQLite.
- Added `PimdirSourceStore::set_collection_name` and the canonical `set_collection_name` behind it (STORAGE §14): `collections.name` was seeded from the id and never written again, so a namespaced id put its prefix in the label too and a DAV collection addressed by a UUID had no readable name at all.

### Changed

- BREAKING: renamed every engine type from `Replica*` to `Pimdir*` (`ReplicaSourceBinding` is `PimdirBinding`, `ReplicaEvent` is `PimdirSyncEvent`), the read placement to `PimdirItemLocation`, and moved the client types under `client::{reader, producer, blobs, diagnostics}` with no crate-root re-export.
- BREAKING: `conventions` is `summary`, and `PimdirDerivation` carries a typed summary instead of a meta string.
- BREAKING: every collection parameter of the std client takes `impl AsRef<str>`; `PimdirCollectionId` implements `AsRef<str>`.
- BREAKING: `PimdirProducer::enqueue` takes no timestamp and takes the body as `Option<&PimdirObject>` instead of a size; `PimdirAction::Add` and `Update` carry no summary, the owner deriving it from the body.
- BREAKING: the change key is the derivation SYNC §4 fixes (FNV-1a 64 over NUL-terminated fields, a flag count in decimal ASCII), so every key changed.
- BREAKING: coroutine states are named in the present tense, and a completed coroutine answers `UnexpectedArg` to every resume, `None` included.
- BREAKING: `sql::ALL` is `sql::CANONICAL`, `sql::OWN` and `sql::all()`; the canonical SQL of pimdir draft-01 is vendored under spec/ byte for byte and its constants generated by build.rs, one per statement file.
- BREAKING: every store from an earlier draft is refused with `PimdirError::Stale` and recreated.
- BREAKING: renamed every `--json` key of the CLI output to camelCase, the `completions` and `manuals` commands to `completion` and `manual`, and replaced the `schema_version`/`supported_schema_version` pair of `store info` with one `schemaVersion`.
- BREAKING: `PimdirMutation::Copy` and `Move` carry no `placeholder`, `PimdirMutation::Add` and `PimdirAction::Add` no `handle`: every create sits under the link id's provisional handle.
- BREAKING: `PimdirArg::LookupObject` and `lookup_objects` answer `PimdirObject`s, hash and size, for the size witness an immutable link needs (SYNC §6).
- BREAKING: `drain_collection` is `drain`, store-wide in append order; `PimdirPendingAction` names its `collection`, `PimdirDrainReport` counts `retried`, and `queued_collections` is gone (STORAGE §15.2).
- BREAKING: `PimdirChangeCursor::changed` is the last stamp drawn rather than the next; `PimdirRetention::at` is `Option<String>`, `None` for a tombstone a source still holds; `PimdirError::Stale` carries the `missing` table or trigger; `rename_collection` takes `&mut self` and retargets the queued moves and copies (STORAGE §4.5, §11, §15).
- BREAKING: the `client` feature links the system SQLite through pkg-config rather than building its own, and the new `vendored` feature, off by default, restores the source build (`rusqlite/bundled`), the way `vendored` covers OpenSSL elsewhere.
- A drop carries three reasons: `Deleted`, `Superseded` (an accepted add's provisional handle, one a landed arrival replaced, or the handle of a vanished edit re-staged) and `Rekeyed` (a rebuild), the last being what bumps the collection's generation.
- A sync's events report what the remote changed and an accepted add only; a pushed flag, body or delete reports nothing.
- A write retires the binding a handle held under another link id before inserting the new one, `bindings_by_handle` being unique (STORAGE §10), and purges a dropped item's row instead of retaining it when another collection of the same account holds the identity live (STORAGE §11).
- A load by key (`PimdirLoadScope::Links`) answers with the rows the source binds, never the copy the hub offers for an item the source lacks.
- A rekey carries a remote edit as a pull, a conflict over a local edit or a revived tombstone, never a base at a revision it never reconciled; it lists every `Rekeyed` drop before every upsert, pairs the copies of one hint by base revision then body, and carries an item-level conflict with no revision (SYNC §8).
- A create waits while the collection holds a probe; a `Full` fetch moves the base, marks a conflict over a staged edit the remote moved past, and clears a conflict whose diverging body is the placement's own; a mutable member restating its hint is keyed afresh; a tombstone holding a staged edit over a remote edit follows the conflict policy; a `Remove` settles a conflict against the recorded revision; an immutable kind's base adopts the shared body (SYNC §5 to §9).
- The hub judges mutability across every binding, reads the divergence against the base the binding held, and keeps a rebuilt binding's base for the upsert that rebinds it.
- The hub adopts a tombstone's known flags, lowers the shared level with a body a pull drops, and asks its resolver for `Tombstone` placements too.
- `PimdirMutate` refuses a probe with `PimdirMutateError::Probed`; `client::PimdirRunError` is public.
- A drain parks a store failure and a move or copy into a collection of no declared kind, retries a busy or I/O one up to `MAX_ATTEMPTS` without stopping the pass, and applies each action as a source binding its item or syncing its collection rather than as the handle draining (STORAGE §15.2).
- `enqueue` pins the body before the row lands, `collect_garbage` recomputes the refcounts before deleting, `held_elsewhere` compares bodies, a revive keeps a retained body a `Meta` fetch carries none of, the trash lists every deleted row, and the schema check covers the triggers (STORAGE §5, §11, §15.1).
- The collector unlinks only files at their shard path, and a blob write syncs every shard directory it created.
- A reader or producer opening a directory with no database reports `Uncreated`; a store lacking a table is `Stale` before its stamp is read.
- Bumped `rust-version` to 1.88 and the export dump `format_version` to 2 (items carry summary columns and address rows instead of a raw meta string).
- The `cli` feature pulls pimalaya-cli through `dep:`, and .github/, cairn/ and tests/proptest-regressions/ are excluded from the package.

### Fixed

- Fixed the `hash:` key using a wrong FNV-1a 64 prime, so no UID-less card or event keyed as the specification says.
- Fixed an accepted content push rebasing the placement read before the flag merge, losing the flag it pulled in the same run (SYNC §5).
- Fixed a re-listed probe with unchanged flags being rewritten and reported on every full sync, and a probe an enumeration no longer lists never being dropped.
- Fixed `PimdirSyncReport::rejected` counting a rejection for a handle nobody pushed.
- Fixed a bound source projecting dirty on every load after a sibling source pulled a content change.
- Fixed an `Edit` on a pending copy keeping its origin, so the origin's body was copied instead of the edited one uploaded.
- Fixed `windows-1252` decoded as latin1, folded header whitespace collapsed, and an encoded word whose payload starts with `=` failing to decode.
- Fixed one malformed pending row failing every overlaid read.
- Fixed single-statement owner writes surfacing SQLITE_BUSY as a raw SQL error, and unknown address roles reading as `from`.
- Fixed the CLI opening a second reader behind the handle a verb already held.
- Fixed CI checking out io-replica and a `conventions` suite that no longer exists; every spec suite is now proven to run.
- Fixed the docs.rs feature badge missing on the `client` module.

### Removed

- Removed `items.meta` and `ReplicaMeta`: the summary is typed.
- Removed the draft reconcile (`FOLDED_IN`, `ENSURE_INDEXES`, `RESHAPED_INDEXES`) and `PimdirError::Unreconcilable`.
- Removed `PimdirStore::open_read_only`, superseded by `PimdirReader::open`, the in-memory residual of unnamed handles, superseded by the probes table, and `write_rekeyed`, superseded by the `Rekeyed` drop reason.
- Removed the `io-replica` dependency, the accidental public `pimalaya-cli` feature, and the duplicated `fs4` and `rusqlite` dev-dependencies.
- Removed `PimdirConflictPolicy::KeepBoth`, `PimdirDeletePolicy` and `PimdirSyncOptions::delete`: a second copy of a mutable resource cannot land on a `UID`-keyed server, and a refused delete is a fact about the collection, not a preference (SYNC §5).

## [0.4.1] - 2026-09-02

### Fixed

- Fixed a write with no link id filing a second row for a handle already bound to an item.

  A placement carries no link id because its identity has not been read yet, not because it has none, and a handle the store has bound already has one. Such an upsert now resolves through that binding and folds into the item it names; only a handle no binding holds still stages unlinked.

  io-replica writes one on every remote edit that resurrects a locally deleted item, so `load` answered with two placements for one handle, the upgrade read that handle's identity twice, and a `dup:` key was minted for a copy nothing holds. The rebind guard skipped such a placement too, and now sees it.

## [0.4.0] - 2026-08-31

### Added

- Added `bindings.shared_object`, what a source last agreed with the hub on.

  A binding recorded one base, the state last agreed with its own remote, and the engine measured two different questions against it: only a sync moves that base, so a body the source itself folded in and has not pushed yet leaves the same gap another source folding in leaves. A second offline edit was therefore filed as a cross-source conflict in a store with one source and no second source anywhere, and so was the edit resolving a conflict, whose merged body then never became the item's. io-replica gave the second axis a base of its own; this is where it survives a restart, which is the only place it matters, the absorb that files a conflict and the edit that settles it being different runs.

  Round-tripped through `ReplicaSourceBinding` and meaningful on every binding, conflicted or not, unlike the conflict pair beside it: gated on the flag it would be erased at the moment a resolving edit needs it. It names an object and pins none, taking no `REFERENCES`, no index and no refcount, being only ever compared for equality and never read as bytes. Folded into schema version 1 under the draft allowance (spec §6) and backfilled from the item's own body, so an upgraded store opens in agreement rather than conflicting once for every item whose push is pending. `item show` prints it beside the base it is not.

- Added `bindings.conflict_object`, the diverging body an unresolved conflict is waiting on.

  A binding recorded the remote revision it diverged at and nothing else, so whoever resolved the conflict had to fetch the body that revision named, which meant credentials in the resolving tool. The column holds that body, round-tripped through `ReplicaSourceBinding` and meaningful only while `conflicted` is set, on the same terms as the revision beside it: base, local and remote are all readable from the store alone.

  It is a reference like any other, counted in the refcount and therefore kept out of the collector for as long as the binding stays conflicted, which is the interval between the run that found the divergence and the day a person sits down to it; resolving is an ordinary edit and releases the pin. The item-level `items.conflicted` and `items.conflict_object`, the cross-source axis, is untouched. Folded into schema version 1 under the draft allowance (spec §6), so a store written before the column is reconciled on open rather than refused. `item show` prints it beside the revision.

- Added `PimdirReader::list_conflicts`, what is waiting for a decision.

  The conflict flag was written and read back with its row and never filtered on, so counting the outstanding conflicts of a store meant paging every collection, on every run. One statement now answers it across an account's collections, naming each binding by its collection, link id, source and handle and carrying the three hashes the divergence is between, and it rides a partial index that holds only the outstanding rows and is empty at rest.

- Added `PimdirReader`, the read role (spec §8).

  The format names three roles and the crate shipped two handles: `open_read_only` returned a `PimdirStore`, the type that also drains the queue, sweeps the objects and purges the trash, so a consumer that only reads held a handle that could destroy the store and "it never calls those" was the only thing keeping it from doing so. The reads move to a handle that takes no lock and carries no write at all; `PimdirStore` dereferences to one, so the projection stays a single implementation whichever role reads it.

- Added the pending-action overlay (spec §15.4).

  A reader built with `with_pending` folds the queue's pending actions over the committed items, so a producer sees what it staged before the owner applies it: `set-flags` and `update` restate an item, `remove` and `move` take it out of a collection, and `move` and `copy` bring it into the target. All of them address an existing item, whose `seq` follows its link id store-wide (spec §9.1), so nothing invents an identifier. A queued create has no `seq` until the owner applies it and is reported apart, by `pending_creates` and `count_pending_creates`. Parked rows never overlay, their error saying they will not be applied without an operator.

  The choice is made when the reader is built, never per call, so one handle cannot answer two ways about one collection. A page keeps its meaning: the fold reads past the limit by the number of pending removals and cuts back afterwards, so a page comes back short only where the collection ends and a scan written to page until a short page does not end early.

- Added `PimdirStore::cancel_action`, cancelling one queue row as the owner while holding that role only for the length of the call.

  Cancelling is an owner write (spec §15.5) and the only retraction a queued create has, the kinds addressing an existing item being retracted by their inverse instead. A consumer that is otherwise a reader and a producer needed the whole owner handle to reach it, which is the handle it must not hold.

- Added `PimdirBlobs::path`, where a body under a hash lives.

  The sharded layout is normative (spec §5) and the format invites a consumer to stream a body straight to that path and index it with a byteless `StoreObject` afterwards (spec §14), which until now meant deriving the sharding a second time.

- Added `conventions`, the per-kind derivations (spec Annex A).

  One `derive(kind, body)` returning the `link_id`, the `meta` summary and the `sort_key` for `message/rfc822`, `text/vcard` and `text/calendar`, in the I/O-free core so a consumer running its own SQLite driver reaches it. Annex A is informative, so nothing reported the divergence three consumers writing it separately had already produced: one wrote a whole calendar summary and a resolved key where another wrote `{"v":1,"etag":…}` and an empty key for every item. The fallback ids are fixed here too (`alt:` for a message with no `Message-ID`, `hash:` for a card or a resource with no `UID`), because two writers disagreeing about one item's id link it twice and store one body twice. Checked against the format's own vectors, fixtures read as bytes and structures compared rather than JSON text.

- Added `pimdir gc`, the collector.

  It drops the object rows nothing references, unlinks their bodies and the orphan blob files a crash left, and reports what it freed. It takes the owner role, so it never runs beside a sync, and reports a producer mid-append rather than waiting on one. A store that wants its unreferenced objects bounded schedules the verb.

- Added the single-owner rule, enforced (spec §8).

  Every owning handle takes an exclusive advisory lock on the store directory and holds it for its lifetime; a second owner gets `PimdirError::Owned` immediately, naming the store, rather than the 30-second `busy_timeout` wait that used to end in a stall with no signal. The rule is about processes, so a two-sided sync or a multi-account owner opening several handles still holds one lock. Readers take none. Producers take a shared one, so several append at once while none of them keeps the owner out, and a body is never between the blob tree and the queue row that pins it while a collector runs. The kernel releases the lock with the process, so a crash leaves nothing to recover.

- Added `PimdirReader::item_bindings` and the bindings `pimdir item show` prints.

  A binding is where a source's own view of an item lives: the handle it is addressed by, the base the last sync agreed on, and the marker saying it diverged. None of it was readable, so the question a duplicated identity raises, which resource does each copy come from, could only be answered by going back to the server or opening the database by hand. `item show` now prints one block per source under each placement, `conflicted at revision` appearing only when it applies. `item list` is untouched: its rows are one query for a whole page, and a binding lookup per row is not what a listing should cost.

- Added the minted-key count `pimdir check` reports per collection, the second copies of an identity a source hands over twice (spec §9).

  Not a defect and nothing to repair, so it is counted apart from the problems: what it is worth saying is that a collection whose count climbs every sync is a source renaming the same duplicate on every run.

### Changed

- **BREAKING** Dropped the source parameter from `PimdirStore::open` and `open_read_only`, and moved the source-bound operations to `PimdirSourceStore`, yielded by `for_source`.

  `PimdirSourceStore` carries the storage seam, the rekeyed write and the queue drain, and dereferences to the store for everything else. Purge, queue cancellation and every read consulted no source but had to be handed one anyway, so the CLI invented `"pimdir"` for its readers and scanned `bindings` for a name it then discarded. Both are gone, and a store an operator reads and sweeps now records no source at all.

- **BREAKING** Deduplicated the crate against itself, with no behaviour attached.

  Fourteen reads that each wrote out prepare-map-push-return share one `rows` helper; the statements and the `sql::ALL` index are declared by one macro, so the two tests that re-read this crate's own source to keep them in step are gone; `PimdirRetainedItem` is folded into `PimdirItem` as an optional `retention`, read by one row mapper instead of two; the operator tool's second read-only connection (`PimdirDb`) is folded onto the store as a diagnostics block; `park` becomes a call to `fail_action`. Breaking beyond the fold: `PimdirPlacement` carries the same typed columns as every other read (`ReplicaLinkId`, `ReplicaFlags`, `ReplicaLevel`), and `PimdirError::Version` splits into `Version` (a schema this crate does not service) and `Uncreated` (no schema yet, which only an owner creates).

- **BREAKING** Bumped the `io-replica` dependency to 0.5, taking the engine's new API with it.

  `load` carries a scope, `DropPlacement` carries a reason, and `ReplicaCollection` is gone. The git patch this crate carried until the engine release that mints a key for a duplicated identity is dropped.

- **BREAKING** Made a purge report the rows it retired, not the bytes it reclaimed.

  It releases the references a retained item held; the bodies are freed by the collector, which is what can report them. `PimdirPurgeReport` loses its `bytes` field and `item purge` its byte count.

- Moved the blob write out of the database's writer lock.

  Bodies land in the blob store before the transaction that indexes them opens, which spec §14 asks for in as many words: inside it, the same write held SQLite's single writer lock across a file write, two `fsync`s and a rename, serialising every other writer behind an I/O path that touches no database page. The queue drain, which builds its ops inside the transaction that claims its row, keeps writing inside it; the blob write is idempotent, so that costs one existence check.

- Stopped the collector from holding every hash in memory.

  It read the whole `objects` index into a set to diff the blob tree against, which is hundreds of thousands of names at the scale spec §1 promises, to answer a question that is always about one file. It asks per file on the primary key now (`OBJECT_EXISTS`, new in the format's statements).

- Made a purge release the pins its own delete reported.

  `PURGE_ITEM` and `PURGE_RETAINED_BEFORE` return each removed row's `object_hash` and `conflict_object`, so the pins are settled from the statement that took them rather than from a read that visits every swept row a second time. `RETAINED_ITEM_BY_SEQ` and `RETAINED_BEFORE` are retired.

- Checked the format's own SQL, and its one MUST vector.

  The fidelity suite prepares every canonical statement against the canonical schema rather than only checking that each is inlined here by name: this crate holds the only toolchain that ever loads those files. `tests/objects.rs` checks object naming against `vectors/objects.json` (spec §16), both algorithms, whole and streamed, shard paths included. Both suites, and `conventions`, skip silently without the sibling spec checkout, so the new CI in this repository and in pimdir asserts they ran.

- Made `check` diagnose and `--fix` repair, neither reclaiming.

  `--fix` used to delete orphan blob files behind a `--grace` window and a confirmation prompt, while every write swept with neither. Both flags are gone: it now recomputes the drifted refcounts from the pointers that justify them and clears the bindings whose item is gone, which destroys nothing and needs no guard. Orphan files are reported, and `pimdir gc` takes them.

- Made `item restore` queue rather than fail when the store is owned.

  It appends its action first and takes the owner role only to apply it, so a restore issued while a sync runs reports `queued` and is drained by the owner that holds the store, which is what the queue is for. Purge and `queue cancel` still need the role and still say so.

- Scoped a write to the rows its batch names.

  Folding a batch into a collection loaded, cloned and diffed the whole collection, so the cost of one flag on one message was the size of the mailbox: measured 3.5 ms at a thousand items, 13 ms at four thousand, 59 ms at sixteen thousand, cleanly linear, against a promise of hundreds of thousands. The read is now scoped to the link ids the batch carries, with each dropped handle resolved through the new `bindings_by_handle` index, and the same measurement is flat at 150 to 175 µs across that range.

- Keyed the residual rather than listing it.

  A first sync probes a whole collection before linking any of it, so the list grew to the collection size while every insertion, drop and lookup searched it linearly.

- Made the drain answer its point questions with point reads.

  The `Add` collision check, the handle lookup and the mutation it drives each loaded the whole collection, once per drained action.

- Made `release_pins` one statement rather than one per hash.

  A purge of fifty thousand retained items was a hundred thousand point updates inside one transaction.

- Made the object sweep test `refcount <= 0`, so a count a double release drove negative is collected rather than leaking for ever with nothing reporting it.

- Added the indexes `objects_garbage` (partial, so the sweep stops scanning the whole table on every write transaction), `items_by_seq_global` (the store-global public id the format promises had no store-global index), `bindings_by_handle`, `items_by_conflict_object` and `queue_by_object`.

### Deprecated

- Deprecated `open_read_only` in favour of `PimdirReader`.

### Fixed

- Fixed a queued action the draining source could not place being parked, destroying it for the source that could.

  Staging an existing item's action resolves that item's binding for the draining source, and no binding was answered with a park: `error` set, filtered out by every later drain, cleared by nothing in the crate. The first source to reach an action it could not place therefore made it unappliable for the source that held the item. On a consumer draining once per source, in name order, over one store, that is every action a frontend queues against a collection whose source does not sort first. The spec already said an action the owner cannot apply here is skipped and left pending, never parked, so that is now what happens: the row is left exactly as found, its `attempts` untouched, and counted as `skipped`. Rows already parked stay parked, `queue cancel` remaining the only exit.

- Fixed a trash page sorting the whole trash to return fifty rows.

  `list_retained_page` moved onto the public `seq` (spec §14.1) and `items_retained` did not move with it, so the read could not ride its own index. An existing store is repaired on open: an index whose columns moved is now dropped and recreated, which the ensure batch could not do, `CREATE INDEX IF NOT EXISTS` keying on the name and leaving the old shape in place silently.

- Fixed a process refusing itself the store it had just released.

  The owner lock is the process's and shared across handles, but a strong count reaches zero before the file description it named is closed, so a handle taken in between opened a second description and `flock` refused it against this process's own: `PimdirError::Owned` naming a store nobody else held, on no schedule, which nothing above can act on. The registry owns the description now and closes it as the last handle goes, inside the same critical section the next acquisition takes.

- Fixed a body stored without a placement being destroyed at the end of the batch.

  Every write swept the object rows at refcount zero and unlinked their blobs, so a consumer that streamed bodies into the blob tree and attached them in a later batch, which spec §14 invites and which `STORE_OBJECT` inserting at refcount zero exists for, lost them silently, bytes included, before the batch that would have referenced them ran. No write collects any more; `pimdir gc` does.

- Fixed a body lookup crossing accounts.

  `lookup_objects` resolved a link id against every collection in the store, so two accounts holding the same vCard `UID`, which spec §9.2 names as a thing unrelated servers do, handed each other's bodies across: the receiving sync then believed the item was hydrated and never fetched the real one. It is scoped to the caller's own account now, which is the axis a link id is trustworthy on; across collections it still answers, which is what the read exists for.

- Fixed a base of no revision, no body and unread markers round-tripping as no base at all, so an agreed placement read as never-agreed and the sync re-derived the same push on every run.

  `bindings.base_present` records the fact its three value columns cannot express; those columns stay a witness for rows written before it.

- Fixed an identity a source held twice being stored once, the write that lost the other silent.

  A binding pins one handle, so a source holding one link id twice (a double delivery, a retried append, a restore, a migration, two DAV resources sharing a `UID`) had nowhere to put the second copy: the write repointed the binding at it, and no layer above could afterwards tell the source held the identity twice, while deleting the bound copy propagated a delete that removed the only copy on a source nobody touched. The second copy is an item of its own now, under the key the engine mints for it (spec §9), with its own `seq`, its own binding and its own body; a write that still resolves a stored binding to another handle is refused with `PimdirError::Rebind` rather than applied, and nothing is recorded in its place. The one licensed rebind stays licensed: a handle-space rebuild supersedes the handle it replaces, per handle, so a renumbered collection carries over and a rebuild holding a genuine second copy is still refused for that one.

- Fixed a write carrying a new sort key silently discarding it.

  The diff that decides whether a row needs an `UPDATE` compared every column the statement writes except `sort_key`, so a key that changed and nothing else reported the row unchanged and no statement was issued. A key is derived rather than given, and a connector fixing its derivation, a tzdb update moving a zoned start, or the second source of a two-source sync all restate one; the item stayed where the first derivation put it, for good. The suite missed it because it only covered the other half of the invariant, that a write carrying no key must leave the stored one alone.

- Fixed a descending page hiding every item sorting above its first cursor.

  "No cursor" was expressed as a key no real one could outrank, but a sort key is arbitrary text a writer derives, so no value is reserved and the sentinel was outranked by two of the same character. Such an item was invisible to every descending page, permanently, while the count still reported it. The statement now says what it means, a `NULL` cursor, and keeps the same keyset comparison and the same index.

- Fixed two owners draining one collection applying every action twice.

  The pending rows are read outside any transaction, and the row was deleted at the end of the applying transaction, so a second owner holding the same list re-applied all of it; `add` and `copy` are not idempotent, and the operator CLI opens a second owner handle routinely. The delete is now the first statement of the transaction (`CLAIM_ACTION`, a `DELETE ... RETURNING id`) and a claim that deletes nothing skips the row: exactly-once is a property of the statement rather than a convention about who runs the drain.

- Fixed a blob rename never being made durable.

  The body was written, `fsync`ed and renamed, and the directory entry that carries the name was not synced, while the SQLite commit is. A power loss could leave a committed row pointing at a body that never arrived, which is the one asymmetry the write order exists to prevent.

- Fixed a flag set the store could not decode reading as a known-empty one.

  That is an authoritative "this item carries no markers", which the merge took as one side's opinion: it cleared every marker the other side reported and persisted the result, turning a read failure into permanent loss. It now decodes as unknown, which holds no opinion.

- Fixed `created_at` holding epoch milliseconds where the column is declared to hold an RFC 3339 timestamp, and the empty string when the clock predated the epoch.

  It is written by SQLite itself now, in the form the retirement clock already uses, which also keeps the crate free of a clock.

## [0.3.0] - 2026-08-24

### Added

- **The store owns the content hash its objects are named by.** `PimdirHashAlgo` implements both algorithms the format admits (`blake3`, recommended, and `sha256-128`) with the encoding spec §5 fixes, lowercase base32 (RFC 4648, no padding), and a store, a producer and the algorithm itself hand it out whole (`hash`) or incremental (`hasher`, for a body streamed into the blob store).

  Until now this crate stamped `store_meta.hash_algo` with `blake3` and hashed nothing, while every Rust consumer carried its own 128-bit FNV-1a rendered as hex and the Android app computed `sha256-128` as base32. The recorded algorithm was therefore false, the digest was not cryptographic (spec §2), the encoding was not the one a blob path is specified to use, and the two implementations of one store named the same body differently: no dedup, no blob found, and nothing erroring while it happened.

  `open_with_hash` declares the algorithm a store is created with, `open` adopts whatever an existing store records, and an open declaring a different one is refused with `PimdirError::HashAlgo`.

  **Breaking**: `PimdirBlobs::open` takes the algorithm too, since the blob directory is what names files by it; `PimdirStore::blobs` hands out a handle already bound to its store, which is how a consumer avoids picking one.

- **A store created before the rename cascades is refused on open**, with `PimdirError::Unreconcilable` naming the table. Every foreign key onto a renamable parent carries `ON UPDATE CASCADE` (spec §14), and no `ALTER TABLE` can add one, so the draft reconciliation cannot reach it; spec §6's other branch is to refuse the store and have the operator recreate it, which costs a resync of what the format calls a derived cache.

  Opened anyway, such a store works until something renames a collection, and then SQLite refuses the rename one dependent row down: a server-side rename or an account rename could never be applied, and nothing said so until one was attempted. Every store created by 0.2.0 is in this state.

- **An unknown flag set is written as `NULL`.** Spec §13 keeps two absences apart: `NULL` means nothing has read an item's markers, `'[]'` means it is known to carry none. io-replica had no unknown state, so this crate wrote at least `'[]'` and a probed placement claimed to carry no markers. `flags_to_json` now returns `None` for `ReplicaFlags::Unknown` and `flags_from_json` decodes a `NULL` column back to it, so the column means what the specification says it means.

  In a queue payload (§15.3) an unknown set encodes as `null` rather than `[]`: an action states an intent, so its set is known in every payload the format defines, and a nonsensical one stays legible instead of reading as a deliberate clearing of every flag.

- **The two schema stamps are checked against each other on open.** Spec §4.2 has `PRAGMA user_version` and `store_meta.version` mirror one another and calls a store where they disagree corrupt; this crate read only the pragma, so a half-applied schema change opened as a store at whichever version the pragma happened to hold. Both the owner and the read-only open now refuse it with `PimdirError::VersionMismatch`. A store whose `store_meta` row is absent is left alone, since refusing there would make a missing stamp unrepairable.

- **A collection can be paged in its kind's own order.** `items` gained a `sort_key` column and an `items_by_sort` index, and `list_items_page_asc` / `list_items_page_desc` return a keyset page ordered by it: newest first for mail, A to Z for contacts, a date range for calendars. Until now the only orderings a store could serve were by `link_id` or `seq`, neither of which means anything to a reader, so every consumer had to scan a whole collection into memory to show fifty rows.

  The cursor is the `(sort_key, seq)` pair rather than the key alone, because a key is not unique: two messages share a timestamp, two contacts share a name. `seq` breaks the tie, which is what stops a page boundary that lands inside a tie from skipping an item or serving it twice. The first page takes no cursor.

  An empty key means unknown and is the default, so an item is orderable before it has been summarised: it sorts to the end of a newest-first listing and to the head of an A-to-Z one.

  `set_sort_key` restates one item's key, for a store written before its kind had a convention or a consumer whose sync engine does not carry the key inline yet. An ordinary write never resets a key it does not carry.

- **A collection can declare which account it belongs to.** `collections` gained a nullable `account` column and the partial `collections_by_account` index (spec §9.2), a handle speaks for one account (`PimdirStore::open(dir, source).for_account("work")`, and the same builder on `PimdirProducer`), and `list_accounts`, `list_collections_by_account`, `collection_account` and `set_collection_account` read and restate it. Without the column, a merged view, whose defining operation is "everything except this account", had to reverse-engineer the owner's naming convention with a `LIKE 'work/%'` over collection ids. Folded into version 1, since the format is still a draft, and reconciled on open, so an earlier-draft store heals rather than failing on a missing column.

  **The account partitions nothing, which is the substance of the change.** Link ids, object hashes and `seq`s keep their store-wide meaning, so two accounts holding one `Message-ID` share a `seq` and one body reaching both is one object refcounted twice: scoping identity per account would compile a mail-shaped policy into a kind-agnostic store, and would leave one link id carrying two short forms. What reports the multiplicity instead is `link_placements` on the identity axis and `object_placements` on the dedup axis, each returning every live placement (`PimdirPlacement`) with the collection and account it sits in, and resolving nothing: a mail view lists them, because two receipts of a newsletter have two read states, while a contact view may offer to merge them.

  There is no `accounts` table: the store records which account a collection belongs to and nothing else, so credentials, endpoints and display names stay with whatever configures the owner. `list_accounts` is therefore what the collections say rather than a configured roster, and an account with no collection yet does not appear in it.

- **`rename_collection`**, which gives a collection a new id and carries its items, bindings, sources, queue rows and child collections with it. Every foreign key onto `collections(id)` is now `ON UPDATE CASCADE`, as is `bindings(collection, link_id)`, which is a parent one level down and refuses the cascade without it.

  This is the only safe way to change an id, and it matters because the obvious alternative is destructive: deleting a collection and recreating it under a new id cascades every item and binding away, turning a rename into a full re-download and discarding staged local changes. A server renaming a folder and an owner renaming an account both land here.

- **A spec-fidelity test suite** comparing the inlined `sql` module against the canonical pimdir specification checked out beside it: the schema semantically (columns, defaults, foreign-key actions and indexes, through SQLite's pragmas rather than by text), the presence of every canonical statement by name, and that every inlined statement prepares against the inlined schema. Statement *text* is deliberately not compared, since the specification permits an equivalent substitution; the three this crate uses are listed explicitly instead. Skips when the specification is not checked out beside this crate.

### Fixed

- **A store created by 0.2.0 was unreadable.** The draft-shape reconciliation on open (spec §6) did not carry `items.sort_key` or the `items_by_sort` index, although both were folded into version 1 after 0.2.0 shipped, so every paged read of such a store failed with `no such column: sort_key`. The column and the index are now reconciled with the rest, and a regression test derives an earlier-draft store from the current schema by dropping each folded-in column, so the next fold is covered without rewriting it.

  Still not reconcilable: a 0.2.0 store carries no `ON UPDATE CASCADE`, which `ALTER TABLE` cannot add, so `rename_collection` on one is refused by SQLite rather than silently orphaning its rows.

- **The inlined schema had drifted from the specification.** `sql::MIGRATION_0001` carried neither the `sort_key` column nor any `ON UPDATE CASCADE`, so this crate was creating stores that did not match the format it implements, and nothing detected it. The point of `sql` is to be the canonical copy a consumer runs on its own SQLite driver, so a silent disagreement is the worst failure it has; the fidelity test above exists so it cannot recur.

### Changed

- **`ItemRow.flags` is `Option<Vec<String>>`**, `null` in the JSON output while nothing has read the markers. Breaking for anything parsing `pimdir item list --output json`.

- **`PimdirItem` and `PimdirRetainedItem` gained a `sort_key` field.** Breaking for anyone constructing them.

- **The sort key io-replica now carries on a placement is bound on write.** `load` returns it, insert and update write it. This reverses the arrangement above, where the key was preserved by the update never naming it: that held only while nothing upstream carried a key, and a `load` that drops it now hands every save an unknown key, which the update would write back, blanking on every sync what the previous one derived. Both halves have to carry the key or neither can.

## [0.2.0] - 2026-08-07

### Added

- **A `pimdir` operator CLI**, shipped from this crate behind the `cli` feature (a `[[bin]]` with `required-features`, so a library consumer never compiles clap or any terminal dependency). It is to a store what `sqlite3` is to a database: `collection list`, `item list` (live or `--retained`, keyset-paged), `item show`, `item export`, `item restore`, `item purge` (one `seq`, `--older-than <DURATION>` or `--all`), `queue list [--parked]`, `queue cancel`, `store info`, `check [--fix]` and `export`, plus `completions` and `manuals`, each rendering as JSON under `--json` with logs on stderr.

  It **never interprets item content**: a store is kind-agnostic, so the tool prints ids, flags, levels, object hashes and the raw meta, and exports bodies byte for byte. Rendering a message or a vCard belongs to himalaya and cardamum.

  Reads open the store read-only, so inspecting a store while a sync runs is always safe. `item restore` goes through the queue as a producer and then drains that collection as the owner, reading the item back to report *applied* rather than trusting collection-wide drain counters; when a sync holds the lock the action stays queued and applies at the next drain. Purge, queue cancel and the orphan sweep have no action kind, so they take the owner role directly, and `PimdirError::Busy` reports as "another writer holds the store lock (a sync is running?)". Destructive verbs confirm (with counts and bytes when the store can price them) unless `--yes`, and refuse to prompt into a pipe or under `--json`. Listings never truncate silently.

  `check` closes two gaps the format leaves open: orphan blob files (a crash may leave one and nothing cleaned them) and refcount or reference drift. Only orphan files are reclaimable (`--fix`, guarded by a `--grace` window because a body is written before the row referencing it); drift and dangling rows are reported, never repaired.

- **The store retains items instead of deleting them.** An item whose last source binding vanishes is now soft-deleted rather than removed: `items` gained `retained_at` (RFC 3339, stamped by SQLite so the crate stays clock-free) and `retained_by`, plus a partial `items_retained` index. The row keeps its `object_hash`, so its body keeps a reference and survives garbage collection. A remote expunge therefore never destroys the local copy, which is what makes a store usable as a backup of a source it does not control. Retention is unconditional: whether a removal is terminal must read identically to every process that opens the store, so it is not configurable. How long to keep, and when to sweep, is the owner's schedule.

  `LOAD_ITEMS` hides retained rows from the sync seam. That is the condition of correctness, not an optimisation: io-replica's storage spec states that the merge reconciles only what `load` returns, so a hidden row is never re-derived, on a delta sync or on a full one. io-replica itself needed no change.

- **Purge, the only true delete.** `purge(collection, seq)` takes one retained item, `purge_retained_before(cutoff)` sweeps every item retired strictly before an RFC 3339 instant the caller computes from its own retention policy. Both release the row's object pin and let the ordinary refcount sweep unlink the body, and both refuse to touch a live item. `list_retained`, `count_retained` and `retained_bytes` are the trash view beside the live reads; `retained_bytes` is an upper bound on what a purge would reclaim, since a body a live item also points at survives.

- **A reappearing link id revives its retained row** (clearing `deleted`, `retained_at` and `retained_by`, adopting the new content, keeping the message's `seq`) instead of colliding on the primary key. One branch serves a source-side resurrection and a client restore alike, so restoring an item is an ordinary `Add` over the values retention preserved: no new action kind, no network.

- **An owner now skips the queue actions it cannot apply.** An unrecognised action kind decodes as `PimdirAction::Unknown { kind, payload, object_hash }`, payload verbatim and body still pinned, and the drain leaves the row pending (counted in `PimdirDrainReport.skipped`) instead of parking it, without blocking the actions behind it. Parking claims an action is permanently unappliable, which is wrong for an intent another owner can perform: this is what lets one queue carry store mutations any owner applies beside capability-bound intents such as a mail submission. Malformed payloads still park.

- `drop_action(id)` removes one queue row, pending or parked, releasing its object pin in the same transaction: one verb for cancelling a queued action and for acknowledging an intent performed out of band. `fail_action(id, error)` records a failed attempt, bumping `attempts` for a transient failure or parking with the reason for a permanent one.

- **The store persists a per-source content conflict.** `bindings` gained `conflicted` and `conflict_revision`, round-tripped through `ReplicaSourceBinding`, so the sync layer's memory of "this source and its own remote diverged, unresolved" survives a restart. Without it the merge re-derived the push its remote had already rejected on every run, never converging, and a client could not tell which items needed a human. Distinct from the item-level `conflicted` / `conflict_object`, which is the cross-source divergence; the two are persisted independently. Carried on `ReplicaSourceBinding` as of io-replica 0.3.0.

  The revision is meaningful only while conflicted (spec §11), so a resolved binding cannot hand a stale one to the next sync.

- A store written by an earlier draft of schema version 1 is now reconciled on open. The columns above were **folded into version 1** rather than added as version 2, the pimdir spec being still `draft`, so `PRAGMA user_version` stays `1`, which means an older store is not detectably out of date and would otherwise fail on a query much later. `init_schema` now adds any folded-in column it finds missing (and any index over one), guarded by `PRAGMA table_info` so it is a no-op for every store after the first open (spec §6's draft allowance). This machinery lapses when the spec freezes its first version.

### Changed

- **BREAKING**: bumped io-replica to `0.3`, whose `ReplicaSourceBinding` carries the per-source conflict this release persists.
- `PimdirAction::kind()` returns `&str` rather than `&'static str`, since an owner-defined kind is carried as data.

### Removed

- `PimdirActionError::UnknownKind`: an unrecognised action kind is no longer an error, so nothing can construct it.

- `sql::DELETE_ITEM`: a per-item hard delete has no caller and no counterpart in the format spec's canonical queries any more. An item no source holds is retained (`sql::RETAIN_ITEM`), and the only true deletes are `sql::PURGE_ITEM` and `sql::PURGE_RETAINED_BEFORE`.

## [0.1.0] - 2026-08-06

### Added

- Initial pimdir store: a SQLite index plus a content-addressed, two-level-sharded blob directory, implementing io-replica's storage seam (load, lookup_objects, write) for one source.
- no_std core reusable without the SQLite client: the canonical schema and statements (sql) and the model-to-column encodings (codec).
- Store-global public ids (seq): one per message, shared across every collection it is filed in, monotonic and never reused.
- Streaming blob ingest and read, so a large body is never held whole; a byteless object write indexes a body already streamed to its content-addressed path.
- Incremental, cross-collection-correct reference counting with blob garbage collection inside the write transaction; a crash leaves at worst an orphan blob, never a row without its body.
- Single-writer serialisation via BEGIN IMMEDIATE and a generous busy timeout, so several same-source handles overlap network while their writes serialise.
- An availability-aware, paginated client read surface (list_items, get_item, count_items, distinct_sources, seq_for_link) projecting the store as a local backend.
- The action queue table and collections.generation as part of the draft v1 schema, with user_version and store_meta.version kept in agreement and a store stamped with a higher schema version refused on open (the spec is a draft: draft stores are recreated, never migrated).
- The action queue (spec §14): PimdirProducer (the single enqueue transaction any non-owner process may run, pinning a pre-written body against garbage collection) and the owner's drain (drain_collection applies each action and deletes its row in one transaction, parking permanently failing actions), plus queued_collections, pending_actions (the read-your-writes overlay) and parked_actions.
- The action payload codec in the no_std core: PimdirAction (add, set-flags, remove, move, copy, update, addressing items by public seq) with a strict, versioned JSON round-trip.
- Collection generations (spec §15): the handle-space epoch on PimdirCollection and generation(), bumped atomically with a rebuild batch by write_rekeyed().
- Read-only store open (open_read_only): opens an existing store with SQLITE_OPEN_READ_ONLY, never creates anything, refuses any other schema version, and exposes the full read surface for frontend processes that must be unable to write.

[Unreleased]: https://github.com/pimalaya/io-pimdir/compare/v0.8.0..HEAD
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
