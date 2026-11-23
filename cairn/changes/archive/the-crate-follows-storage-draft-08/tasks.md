---
cairn: tasks
change: the-crate-follows-storage-draft-08
---

## Decisions (proposal.md)

- [x] 1 to 9 taken as recommended by the user, 2026-10-31

## Phase 0: vendored vectors (S)

- [x] spec/vectors/ at pimdir 9606b43, spec/REVISION naming it; Cargo.toml `exclude` keeps spec/vectors/ and spec/payloads/ out of the package
- [x] tests/vectors_sync.rs, tests/summaries.rs, tests/search.rs and every other reader of ../pimdir/vectors read spec/vectors/; no `skipped:` line left
- [x] tests/spec_drift.rs compares spec/ with `git -C ../pimdir show <REVISION>:<path>` when a checkout exists (read-only), else nothing
- [x] Every suite green; log; spec-fidelity.md folded

## Phase 1: the store opens storage-draft-08 (L)

- [x] spec/ re-vendored at d671bbd (migrations/, queries/, vectors/, payloads/), spec/REVISION d671bbd; spec_drift checks spec/vectors/ and spec/payloads/ against vectors/manifest.json
- [x] tests/pending_vectors.txt seeded from the first run, each line a case and its phase; the runners fail on an unlisted failure and on a listed pass
- [x] src/client/schema.rs: the open contract of STORAGE §6 (marker, then `hash_algo`, then the schema check); `rebuild`, the backfills, `as_reconciled`, `keyed_arrays`, `array`, `rederive_task_capture` and the draft-06 lists deleted; callers in producer.rs, capability.rs and write.rs run the statements as they are
- [x] `PimdirError::Version` (`store-version`); `Shape`, `NotCurrent::Shape` and the stale-store paths reconciliation alone needed removed; src/cli/mod.rs maps the codes
- [x] tests/earlier_lists.rs, older_task_summary.rs, store_shape.rs, tests/fixtures/storage-draft-06.sql deleted; store_version.rs, stale_store.rs and open_current.rs rewritten as refusals (a draft-07 store, a store without `format_version`, a `storage-v1` store), each refusing on every profile with nothing written
- [x] src/hash.rs: `PimdirHashAlgo` and `Sha256_128` removed, BLAKE3 alone, `sha2` dropped from Cargo.toml; `init_store_meta` bound without `:hash_algo`
- [x] `load_store_meta` and `load_change_cursor` read the new columns; `raise_incarnation` exposed; `store info` prints the id, the incarnation and the versions
- [x] `set_derivation` sets the revision; `sync_version` written with the first sync write and refused as `PimdirError::SyncVersion` when another draft's
- [x] Pragmas: `synchronous = FULL` and `secure_delete = ON` on every writing connection, `fullfsync = ON` on macOS, a busy timeout on readers and producers
- [x] Statements: the six `load_<kind>_summaries_by_link`, `load_in_reply_to_by_link`, `load_categories_by_link`, `load_addresses_by_collection`; `:scoped` gone from src/client/write.rs, src/sync.rs, src/collection.rs, src/search/coverage.rs
- [x] `seq_for_link_any` with `:collection` (src/client/write.rs, src/client/attachments.rs); `list_unstamped_bindings` with `:narrowed` 0, `:enumerated` 0 and its `handles` slot until phase 5
- [x] `count_mail_by_day`'s shift in minutes, its reader API and CLI flag; `list_address_placements` paged by its four-part cursor in src/client/reader.rs and src/client/search/compile.rs (the CLI has no such flag: the reader API alone changed)
- [x] Index statements as re-signed: `init_index_meta`, `load_index_meta` (twelve columns), `set_index_cursor` without `store_reconciled`, `coverage` with its slot, `placements_gone` with `link_id`; `store_reconciled` gone from src/sql/search.rs, src/search/refresh.rs, src/client/search/refresh.rs, tests/search.rs and tests/search_pure.rs, the purge count alone reconciling
- [x] Non-vector suites green; log; store.md, spec-fidelity.md, search.md folded; CHANGELOG (reinstall first)

## Phase 2: the blob store (M)

- [x] src/client/blobs.rs: `objects/<hash[0:2]>/<hash>`; a name checked before any path; the no-replace write (`hard_link` then `remove_file`, an existing file kept); no link followed under objects/
- [x] The collector unlinks only a regular file, by `symlink_metadata`, whose well-formed name begins with its shard's two characters, the shard a directory and no link; anything else left and reported by `pimdir check`
- [x] Verification before a body is served, parsed, pushed or copied; a streamed read failing at its end before its consumer commits; a range read verifying the whole file through its descriptor first
- [x] quarantine/: a body failing its name moved by the owner under the exclusive staging lock, a suffix unique to the move, never unlinked; a `Full` item holding it fetched again; an `add` or `update` carrying it retried then parked
- [x] The drain reads `queue.object_hash` and parks a payload whose `object` differs
- [x] tests/objects.rs on objects.json `v` 2; tests for the no-replace write, a link at a shard and a blob, a malformed name, the quarantine move
- [x] Log; store.md folded; CHANGELOG

## Phase 3: Annex A revision 3 (L)

- [x] src/summary/mail.rs `instant`: comments removed, the day of the week skipped, five words, optional seconds, obsolete years and zones, military letters but `J`, `mm` at most 59, a missing zone or trailing words unparseable, the day existing, 60 seconds read as 59, `30 Feb` and `24:00` `NULL`
- [x] `strip_angles` and `message_ids`: comments skipped, a `<` in a comment ignored, a bracketless id only without whitespace, `<`, `>`, `"` and with one `@`
- [x] `decode` and `charset_decode`: a malformed word or an unknown charset kept literal, a `Q` word's stray `=` refused, `iso-8859-1` and `us-ascii` as windows-1252, only spaces and tabs trimmed, whitespace between decoded words alone removed; `client` enables `charset-tables` (decision 5)
- [x] `mailboxes` and `unquote`: a comment read as a space, each quoted-string its content anywhere in a display name, a group listing its members
- [x] src/summary/calendar.rs `resolve`: the offset before a gap; a `TZID` without `VTIMEZONE` through `jiff`'s bundled tzdb (decision 6); the 1,000,000-period rule bound
- [x] The walk: 10,000 entities beside `MIME_DEPTH_MAX`, 1,000 stand-ins (src/summary/mail.rs, src/summary/mail/attachments.rs, src/search/extract.rs)
- [x] src/summary/contact.rs `KIND` and `ORG`; calendar organizer and attendees, a journal: the three new summaries cases
- [x] `DERIVATION` 3; an owner opening a store at 1 or 2 re-derives held mail and tasks through `list_held_mail` and `list_held_tasks` before recording 3; one at a later revision records 3 with its first derived write
- [x] tests/summaries.rs: 53 cases, their lines off the pending list; tests/deep_nesting.rs still passes
- [x] Log; summaries.md and store.md folded; CHANGELOG (alt: keys of bracketless or commented ids move)

## Phase 4: payloads and capabilities (M)

- [x] src/codec.rs: each action and intent parsed as payloads/ defines it (`deny_unknown_fields`, `flags` and `object` required where required, `rcpts` non-empty, `partstat` closed, an intent's `seq` or `link_id`); the valid `submit` was refused for naming no `object`, which the schema does not define: the message is the row's body
- [x] The producer refuses a malformed payload at the enqueue; the drain parks one, naming the member; the owner's backstop parks an intent its declared source cannot perform
- [x] A test running spec/vectors/payloads.json through the codec: every valid payload parsed, every malformed one refused; the 13 phase-4 lines off the pending list
- [x] `calendar.component.vtodo` in src/capability.rs, src/client/capability.rs, src/client/reader.rs, tests/capabilities.rs; every other name already Annex B's, `capability::is_name` checks the grammar
- [x] A producer strips `X-PIMDIR-` from a resource it did not author (`summary::strip_instructions`, `PimdirProducer::enqueue_foreign_body`); `SCHEDULE-AGENT=NONE` written on every `ORGANIZER` and `ATTENDEE` lacking `CLIENT` or `NONE` when the user accepts that nobody is notified (`summary::calendar::unschedule`)
- [x] Log; store.md folded; CHANGELOG

## Phase 5: the engine on sync-draft-07 (L)

- [x] src/remote.rs: `PimdirRemoteSnapshot::narrowed` and `enumerated`, documented per protocol for the connectors
- [x] The narrowed absence rule: `list_unstamped_bindings` bound with `:narrowed`, `:enumerated` and the slot, through `PimdirLoadSelection::LastPage`; a narrowed round its last page opens is opened in a write of its own first
- [x] The empty-listing park for a round and a rekey (src/sync.rs, src/rekey.rs), `PimdirParked::guard` taking `named`
- [x] A bodiless revive of a mutable kind at `Meta`, the retained body kept over the reviving handle, `Overruled`; an immutable kind keeping it as its base only when the size agrees
- [x] The mail agreement against every held row (pending create, live item others or none bind, tombstone, retained row on size); an `Add` over a disagreeing tombstone or retained row minted
- [x] A divergence over an unread body recorded `Manual`; the upgrade settling it by the source's policy (src/upgrade.rs); the fill settling a `prefer-` item conflict, the body not adopted kept
- [x] `PreferLocal`: the observed body kept as a retained item in a write committed before the push, the base moved onto it, the local body pushed gated on its revision
- [x] Decision 7: `PimdirLoadSelection` (and `WantsLoad`'s `selection`)
- [x] tests/vectors_sync.rs reads `v` 2 (`narrowed`, `enumerated`, `carries`), refusing another `v`; vectors 04, 15, 16, 47, 61 and 104 to 117 pass, off the pending list
- [x] tests/engine/ passes, any changed assertion named in the log (the two property oracles confirm a parked full round)
- [x] Log; sync.md, upgrade.md, rekey.md and seam.md folded, with store.md and hub.md; CHANGELOG (the connector-facing fields first)

## Phase 6: the index on search-draft-08 (L)

- [x] src/search/query.rs: the three limits from SEARCH §8.3 (`LENGTH_MAX`, `GROUPS_MAX`, `TERMS_MAX`), `limit-exceeded`, the pass order; `NESTING_MAX` gone, the descent recursing per group alone
- [x] `OCCURRENCES_MAX` 10,000, the 1,000,000-period bound (`PERIODS_MAX`), `set_cut` (src/search/occurrence.rs, refresh.rs); `cut_before` in the coverage (`PimdirSearchCoverage::cut`) and the coverage the CLI prints
- [x] `index_meta`: `format_version`, `shape`, `floating_zone`; `PimdirError::IndexVersion`; the ordered rebuild; a rebuild on another store id, incarnation or a cursor above the store's stamp, at open and at refresh; `set_floating_zone` and a configured zone (`PimdirIndexRefreshOptions::floating_zone`, `--floating-zone`, `PimdirError::IndexZone`)
- [x] The `missing` retry: `insert_missing`, `load_missing_items`, `delete_missing`
- [x] The thread prune through `unparent_messages`, `prune_messages`, `thread_rows`, `delete_thread`; `DELETE_MESSAGE` and `DELETE_THREAD_UNUSED` removed from src/sql/search.rs, with `LOAD_UNREAD_BODIES`, `LOAD_ITEM_IN_REPLY_TO` and `LOAD_ITEM_CATEGORIES` (canonical `load_in_reply_to_by_link`, `load_categories_by_link`)
- [ ] `coverage` on `items_bodiless` with its slot; `match_object`, `match_summary`, `match_objects_ranked` where a page is walked: `coverage`, `match_object` and `match_summary` landed (a page's hits probed for what they matched), `match_objects_ranked` is run by no answer: relevance orders the whole hit set, bodies and summaries together, in one statement (a departure in search.md)
- [x] The I/O-free refresh (src/search/refresh.rs) yields the new reads and ops (`WantsMissing`, `WantsThreadRows`, `PutMissing`, `DropMissing`, `PruneThread`, `SetFloatingZone`, `ReplaceOccurrences::cut`); src/client/search/refresh.rs answers them; tests/search.rs `memory::a_consumer_of_its_own_shape_folds_the_canonical_index` compares row for row, cuts, missing records and the floating zone included
- [x] tests/search.rs on queries.json `v` 2 (168 cases, `cut` compared), the search store's items 26 to 28; off the pending list, which is empty
- [x] Log; search.md folded; CHANGELOG

## Phase 7: security, erasure, conformance (M)

- [x] Directories `0700`, files `0600` (pimdir.db, index.db, the locks, `-wal`, `-shm`, SQLite copying the database's mode, checked with bundled and system SQLite); lock files opened without following a link (decision 9; Windows by reasoning alone); a store others can read or write refused as `PimdirError::Exposed` (`store-exposed`), the directory a store is created in restricted (src/client/modes.rs, src/client/lock.rs)
- [x] An operation the user empties quarantine/ with (STORAGE §7): `PimdirStore::empty_quarantine`, `pimdir quarantine empty`; quarantine/ created `0700` with the rest
- [x] A store from elsewhere opened with `trusted_schema = OFF` and `SQLITE_DBCONFIG_DEFENSIVE`: every connection, whoever created the store
- [x] src/cli/export.rs and any path built from a name: no path of the crate is built from a name (export numbers its files, a body named by its checked hash); `security::path_component` for consumers, the dump created `0700`/`0600`
- [x] Error text scrubbed of credentials (STORAGE §9.2, §18): `security::scrub` in `PimdirStore::fail_action`, the one column error text reaches
- [x] Decision 8: `PimdirStore::erase`, `PimdirIndex::erase`, `pimdir item erase`; tests/erase.rs proves the key, subject, address and body words leave both files and their WAL, failing without `optimize_text`, `secure_delete` or either checkpoint; the thread prune's gap it found (a pruned thread left with no row kept its row) fixed in src/search/refresh.rs
- [x] README: the conformance statement and the reinstall note
- [x] tests/pending_vectors.txt empty (since phase 6); the helper still fails on an unlisted failure (proved on a scratch copy)
- [x] The level-0 mapping phase 1 noted (src/codec.rs, src/placement.rs) gone
- [x] CHANGELOG [Unreleased] read as a net diff; status landed; log

## Followers (outside this change)

- [ ] pimdir: audit-2026-10-31 Followers lines ticked as each phase lands (pimdir's to edit)
- [ ] pimdir's CI runs this crate's vector runner at a pinned revision
- [ ] neverest: connectors state `narrowed` and `enumerated`; `PreferLocal` and its one-way mode keep the body they push over; its `submit` performer reads the message from the row's `object_hash`, the payload naming no `object` (phase 4)
- [ ] himalaya, pimalaya-cli: the API breaks (`PimdirHashAlgo`, the error codes, the paged person axis, the day shift in minutes, `PimdirLoadSelection` and `WantsLoad`'s `selection`, `PimdirSourceStore::upgrade`'s `PimdirUpgradeOptions`, `PimdirParked::guard`'s `named`, `PimdirRemoteSnapshot`'s two fields in a struct literal, `PimdirSubmit::to_action(message)` and a required intent `source`, `enqueue_foreign_body` for an invitation added from a message)
- [ ] The Android app: its platform-sqlite change, BLAKE3 and one shard level, `queue.object_hash`, the capability rename, its index shape
