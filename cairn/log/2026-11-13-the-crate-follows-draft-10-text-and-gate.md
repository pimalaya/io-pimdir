---
cairn: log
change: audit-2026-11-12
landed: 2026-11-13
---

# The crate follows draft 10, the text rules and the gate

spec/ is pimdir at ff7c883, the third batch of storage-draft-10, sync-draft-10 and search-draft-10: one statement is new (`retry_action`), one changed its text (`evict_before`), the sync vectors moved to format `v` 6 with cases 150 to 154, case 48 was restated, and vectors/payloads.json gained seven cases (73). The migrations did not move, so no store is refused that opened before. The copy was taken from the commit (`git archive ff7c883`), never from the standard's working tree, where a search batch was being written. tests/pending_vectors.txt is empty: every vendored case passes.

This entry also lands the audit's lines that are the crate's alone and waited for this text: T4 (the tools), C6 and F17 (the batch and what an enqueue answers), P4, and what V2's judgement lacked.

## What followed

- **The sync vectors at `v` 6.** `PimdirMutation::Abandon { handle }` gives up a delivered create on a person's request: refused on anything but a delivered create of the run's source (`PimdirMutateError::NotDelivered`, `create-not-delivered`), it writes a `Deleted` drop of the provisional handle and nothing else, pushes nothing, and reports `PimdirSyncEvent::Abandoned(handle)` (vectors 150, 151). Vector 152, a tombstoned delivered create revived by a bodiless mutable arrival, and 153 and 154, `if_match` refused over an item holding no body, passed as the crate stood: `land` already left the tombstone only when the arrival was read, and both refusals compared `Some(read)` with the body held. Case 48 passes restated and left the pending list.
- **A known action stating no usable `v` is malformed** (STORAGE §15.3). The previous follow read it unknown, by the letter of the text then; it is reversed. `codec::payload_outcome` answers `Malformed`, and `action_from_payload` `UnknownVersion(None)`, for a known action whose payload states no `v`, or a string, a fraction, `null` or a boolean; an integer other than 1 stays unknown, as does anything of an action the crate holds no schema for. The drain parks such a row.
- **`retry_action`** (§15.5): `PimdirStore::retry_action(action)`, by handle or bare id, and `pimdir queue retry`.
- **The unconfirmed intent** (§13, §15.2, §15.4, §15.5, Annex B.2): `PimdirStore::unconfirm_action(id, detail)` writes the reserved prefix itself; `PimdirActionStatus::Unconfirmed` and `PimdirParkedAction::unconfirmed()` read it; the CLI shows it in `queue status` and `queue list --parked`. No drain lists a parked row, so nothing retries it.
- **Several actions in one transaction** (§15.1, §15.2). `PimdirProducer::enqueue_batch` appends a batch all or none, one gate per action. The drain applies runs of at most 64 rows (`PimdirSourceStore::RUN`), one transaction each, a savepoint per action, the claim first, `ROLLBACK TO` then `RELEASE` before a park or a retry is recorded in the run's transaction.
- **What an enqueue answers** (the audit's C6): every enqueue answers `PimdirEnqueued`, the handle and the partial supports its one gate computed. `enqueue_with_body_checked` is gone, and `PimdirSearchTagPlan::stage`, which gated every action then enqueued each, is one batch.
- **local/** (§3, §6, §18): `PimdirStore::local_dir()` creates it `0700` on demand. The collector walks objects/ alone. The owner's open refuses one another user can reach. The crate keeps nothing of its own beside the store: pimdir.db, objects/, quarantine/, owner.lock, objects.lock, index.db and index.lock are all the standard's files.
- **A lock file stays** (§8): `PimdirLock::verify`, fstat against lstat, before every write transaction of an owner and before a producer's enqueue, `PimdirError::LockReplaced` (`lock-replaced`) and nothing written.
- **T1 and T5 are the standard's text now.** The crate implemented both in 3c10177 ahead of it. Read against the text: the temporaries are every period-prefixed regular file of objects/ and of its shard directories, read by `lstat`, and a period-prefixed name that is no regular file is left and reported, as §5 steps 4 and 5 say; the three flushes are where the text puts them. Two things the text asked that the crate lacked: the staging lock is now held across an object row written for a body the action does not name, and a producer naming a body by its hash alone flushes that body's shard directory before its row commits. The "departure" notes are gone from cairn/spec/store.md.
- **The released open rule** (§4.2, §18): `PimdirStoreVersion::opens` judges a release by the six columns. It gained `format`, `hash_algo` and `user_version`, and the marker must be `storage-v` followed by the store's `version` (§4.2: the part's major is the schema version, and `set_store_version` writes `'storage-v' || :version`). The draft branch is unchanged, and no open runs the released one yet.
- **The tools** (the audit's T4): `PimdirReader::backup`, `PimdirStore::restore`, `PimdirReader::check_deep`, and `pimdir store backup`, `store restore`, `check --deep`.
- **P4**: a write ensures a collection once per batch and reads an upserted handle's link once. A load still resolves its handles one statement each: no canonical statement reads the links of a slot of handles (`list_unstamped_bindings` and `stamp_bindings` are the two reading the `handles` slot, and neither is that lookup), and no statement text of the crate's own was added.
- **`evict_before`** keeps a delivered create, by the vendored text alone.

## Behaviour chosen where the text was open

- **A failure text beginning with the reserved prefix.** `fail_action`, and every park of the drain, records it behind `failed: `, whole, so the row reads parked; only `unconfirm_action` writes the prefix. The alternative, refusing the text, would leave the row pending and have the drain perform it again, the one thing the prefix exists to prevent; and a server's error relayed verbatim is not the caller's to choose. A performer that wrote the prefix through `fail_action` by hand loses the state, not the text.
- **Which rows may be unconfirmed.** Any intent, an application's own included; a row of an action the owner applies itself is refused (`action-not-intent`), its apply being a transaction. Annex B.2 says `submit` alone ends so among its intents, which is a performer's rule and is not enforced here.
- **`v` written `1.0`.** The standard parked the question. The crate keeps serde_json's reading: `1.0` and `1e0` are floats, so no integer, so malformed for a known action.
- **A batch's `add` is always checked for its key**, the one it states or the one its body in hand derives, where the plain `enqueue` checks none.
- **An owner's open refuses a local/ another user can reach**, or a link or a file under the name; a reader and a producer ignore it, as §3 says they do. §18's refusal names the directory and pimdir.db alone.
- **A lock found replaced is given up for good**: every later check fails, the file put back or not.
- **A restore.** §8 says the files are put back and the incarnation raised. Chosen: bodies and quarantined files are copied in and one already held under its name is kept (a file is never replaced, §5), so what the replaced store alone named stays as orphans for the collector; local/ is replaced whole; the database is staged under local/, its incarnation raised there, then renamed onto pimdir.db after the old one, its WAL and its journal are removed, so the store never exists under the backup's incarnation; the restored database takes the journal mode of the store replaced, WAL when there was none; the lock files and index.db are left alone. A restore is refused while a handle of this process is open.
- **A backup** leaves out a writer's temporary, a link and anything that is not the store's, and its database is one file in a rollback journal mode, which is what `VACUUM INTO` writes.
- **The deep check reports and moves nothing**: it runs as a reader, and a failing body is an owner's read to set aside.

## Problems found in the standard's batch

- **Nothing holds the reserved prefix.** §13 says no row in another state carries `unconfirmed: `, and `park_action` takes any text: an owner relaying a failure's text verbatim forges the state, and the standard does not say what an owner does with such a text. The crate's answer is above; a `CHECK` cannot tell the two parks apart, so this is a rule for §15.2 to state.
- **A restore's details are unwritten** (§8): whether bodies the replaced store alone held go, what a name already held does, the journal mode of the restored database (a `VACUUM INTO` copy is no WAL database, and §8 has the mode chosen at creation), and that the incarnation must rise before the database takes its name, without which a crash leaves a store under the backup's own incarnation.
- **A backup and the temporaries** (§5, §8): §8 copies objects/, §5 calls every period-prefixed file there a writer's temporary; whether a backup copies them is unsaid.
- **The flush owed by a producer that writes nothing** (§5, §8): T2 has it verify a body it names without writing, T5 has "a writer keeping a name it found held" flush the shard; whether the first is the second is unsaid. The crate flushes.
- **Annex B.2 and an application's intents**: "`submit` alone can end with its outcome unknown" is said of the intents the annex lists; an `x-` intent's performer is left without a rule.

None is a case the vectors contradict, and none is on tests/pending_vectors.txt.

## Guards

Seen to fail with the behaviour removed in the working tree for one run, then put back from a copy kept beside it (never through git):

- the rollback to the savepoint: tests/item_conflict_settle.rs `an_action_parking_after_it_wrote_is_undone_alone_in_its_run`, with `an_add_superseding_a_conflicted_item_parks_whole` and two others;
- `PimdirLock::verify` answering `Ok`: tests/store_text.rs `an_owner_whose_lock_file_was_replaced_writes_nothing_more`, `a_producer_whose_staging_lock_was_replaced_enqueues_nothing`;
- `land` leaving a tombstone on an unread arrival: sync vector 152;
- the `failed: ` guard: tests/queue_text.rs `a_failure_text_never_forges_the_reserved_prefix`.

Seen to fail on the crate as it stood: the five malformed payload cases of vectors/payloads.json, sync case 48 (as "passes, take it off the list"), and tests/queue.rs `an_unknown_payload_stays_pending_untouched_and_listed`, which stated the reversed rule and was rewritten, with `a_known_action_stating_no_usable_version_parks` beside it. New behaviour, with no earlier code to fail on: `Abandon` (vectors 150, 151, tests/store_text.rs `a_delivered_create_is_abandoned_on_request_alone`), the retry and the unconfirmed state, the batch, local/, the copies and the deep check (tests/queue_text.rs, tests/store_text.rs, tests/cli_copies.rs, src/version.rs `a_release_judges_a_store_by_six_columns`). Not checked that way: `an_eviction_keeps_a_delivered_create`, whose old behaviour is the previous revision's statement text. Not provable by a test: the flushes, and that a backup's consistency never depends on luck, the threaded test exercising it without a fault injected.

## The statements at 60,000 placements

tests/statement_scale.rs, ignored by default, builds a store of 60,000 mail placements in one collection through `PimdirSourceStore::write` and times five canonical statements, the vendored text beside the text of pimdir 88a24f9 (the revision before the audit's P1 to P3 rewrote them, read from its git history into a scratch directory, `PIMDIR_BEFORE`), both answering the same rows. Release build, the bundled SQLite (3.54.1), median of 9 runs, one machine with other work running: indicative only.

| Statement | pimdir 88a24f9 | vendored (ff7c883) | rows |
|---|---|---|---|
| `list_owing_bindings` | 28.4 ms | 27.6 ms | 0 |
| `list_superseded_bindings` | 18.8 ms | 0.001 ms | 0 |
| `list_superseded_pending` | 3.5 ms | 0.001 ms | 0 |
| `collection_sources` | 2.3 ms | 0.003 ms | 1 |
| `count_unread` | 2.1 ms | 1.6 ms | 1 |

The store took 4.3 s to write, a thousand placements per batch. Half the messages carry a keyword and one in thirteen a second, every base flag set equal to its item's, so no binding owes. Two runs agreed within 1%. The two superseded statements and `collection_sources` gain what the audit measured. `list_owing_bindings` does not here: the audit read 120 to 140 ms against 24 to 25 ms on SQLite 3.51.2 and 3.18.2, and on this SQLite the earlier text, its two `EXISTS (… EXCEPT …)`, runs as fast as the two anti-joins. With one keyword in thirteen messages both read 17.3 and 16.4 ms. The rewrite is kept for the floor it was measured on.

## Left

- The pin in pimdir's checks/io-pimdir.rev, which moves once this lands; checks/implementation.sh then finds the pending list empty.
- No CLI verb stages an `Abandon`: the item verbs enqueue, and an `Abandon` has no queue action. An app stages it through `PimdirSourceStore::mutate`.
- The released branch of the open is still run by nobody, and the opener's schema text check still asks 0001_init.sql to the word (cairn/spec/store.md, "The open judgement is one pure function").
- The indexer does not check index.lock as an owner checks owner.lock.
- A migration runner (STORAGE §6, V1's rebuild rules): the crate runs no migration while the format is a draft.
- neverest, the Android app and the CLIs follow: `PimdirEnqueued` where they read a handle, `enqueue_batch` where they enqueue several, `unconfirm_action` where they wrote a free text, their own files moved under `local_dir()`, `retry_action`, and `Abandoned`.
