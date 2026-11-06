# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/pimalaya/io-pimdir/compare/v0.9.0..HEAD
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
