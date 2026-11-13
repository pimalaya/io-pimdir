---
cairn: log
change: audit-2026-11-12
landed: 2026-11-13
---

# The crate follows draft 10, the payloads and identity

spec/ is pimdir at e4faa21, the second batch of storage-draft-10, sync-draft-10 and search-draft-10: its queries/ (three statement texts, signatures.json), payloads/ (two schemas) and vectors/ (payloads.json at `v` 2, sync/ at `v` 5 with cases 141 to 149, twelve fixtures and twelve summary cases) moved, its migrations/ did not. The copy was taken from the commit (`git archive e4faa21`), never from the standard's working tree, where a third batch was being written. The draft names, the schema and Annex A's revision do not move, so no store is refused that opened before.

## What followed

- **A payload is valid, malformed or unknown** (STORAGE §15.2, §15.3). `codec::payload_outcome` answers one of three (`PimdirPayloadOutcome`). `codec::action_from_payload` keeps an unknown payload verbatim as `PimdirAction::Unknown`, read no further: a kind the crate holds no schema for, or a known one at another `v` than 1, none stated included. It was an error (`UnknownVersion`) every caller had to tell apart, and a known action stating no `v` was parked. The drain leaves such a row pending and untouched and runs no gate on it; every pending read lists it, where a row at a later version was left out.
- **`set-flags` states `base`** (§15.3, SYNC §7). `PimdirAction::SetFlags::base` and `PimdirMutation::SetFlags::base`: with it the owner applies the change alone (`PimdirFlags::changed`), refused over an unknown set (`PimdirMutateError::UnknownFlags`, `flags-unknown`), which parks. The capability gate reads it against the current set.
- **`remove` states `if_match` and `resolves`** (§15.3, SYNC §7). `PimdirAction::Remove` and `PimdirMutation::Remove` are struct variants carrying both. A remove is refused over another body (`Replaced`), over a conflict it does not name and over none it names (`Unresolved`), as an edit is; resolving its binding's conflict the base adopts the conflict's revision and body, and resolving the item's it states `Settle` before its tombstone.
- **A tombstone settles nothing by itself** (STORAGE §14, SYNC §9). `PimdirHub::absorb` cleared the item's conflict on every tombstone upsert. It clears it only when a `Settle` precedes the upsert. This reverses what the crate did since the hub held conflicts.
- **The reader overlay** (§15.4) folds a `set-flags` stating `base` as a change, takes nothing out for a `remove` the owner would park, and lists unknown rows. The search tag plan states the set it read as `base`.
- **The agreement across collections** (STORAGE §9, §9.1, §11.1; SYNC §6). `lookup_objects` answers one `PimdirHeldObject` per holder with the row's `subject`, `date` and `sender`, and the upgrade links only from a row agreeing with the placement's summary and, where it states one, on its size. `seq_for_link_any` is bound with the three from the summary being written. `held_elsewhere` is the new text under the same parameters.
- **`Disagreed`** (SYNC §5, §6). `PimdirSyncEvent::Disagreed { handle, link_id }` after the `Added` of a member minted apart from a pending or a delivered create of the source.
- **Summaries.** A `UID` empty once trimmed of the space and the tab is keyed by the `hash:` fallback (`summary::uid_key`). The first override, the trimmed field name, the first occurrence, the fold and the trim were read so already: `str::to_lowercase` applies Final_Sigma and `str::trim` the White_Space property, which the two new card cases confirm.
- **The charset tables are no feature** (Annex A.1, the audit's D4). encoding_rs is a plain dependency and `charset-tables` is deleted, no alias. encoding_rs is `no_std` with `alloc` (its lib.rs: `#![cfg_attr(not(feature = "std"), no_std)]`), so the no_std core keeps building. Keeping the feature meant a build deriving other subjects and `alt:` keys silently, or refusing to derive, which is worse. Cost: none for a build with `search`, which linked encoding_rs through mime-rs's `encoding` feature already (`cargo tree --no-default-features --features search -i encoding_rs`), and none for `client`, which enabled the feature. Only a build of the bare core gains the tables; its size was not measured.
- **The extraction reads a field from its first occurrence** (SEARCH §6, §9; the audit's D5): `Subject`, the address fields, `In-Reply-To` and `References`. cairn/spec/search.md said "last" as a SHALL, and tests/search_pure.rs locked it; spec, code and test moved together.
- **The extraction's attachment test is A.1's mark** (D10): any disposition type but `inline`.
- **`PimdirSyncReport::waiting`** (Y7) is a list of `PimdirWaiting { handle, reason }`: `OutOfScope`, `Delivered`, and `Undeliverable` for a create holding neither body nor origin, which nothing counted. No CLI verb printed the count.
- **An enqueue answers a handle** (C4): `PimdirActionHandle`, the id under the store's id and incarnation, `Display` and `FromStr`. `action_status` reads `Reset` and `drop_action` and `cancel_action` refuse (`PimdirError::Reset`, `store-reset`) when the store's identity moved; a bare id is followed unchecked. `pimdir queue status` and `queue cancel` take either, and `item restore` and `tag` print `handle` beside `row`.

## The seam

Three things moved, all in cairn/spec/seam.md: `PimdirArg::LookupObject` carries `BTreeMap<PimdirLinkId, Vec<PimdirHeldObject>>`; `PimdirLoaded` gains `offered`, the `Created` placements the store offers a source that binds not their item (`PimdirHub::offered`), without which the sync cannot tell its own pending create from a copy another source's item projects, and vector 93 would report a `Disagreed` it must not; and the `seq` decision stays in the storage's write, which reads the three fields off the upsert's summary, so no yield carries it.

## Where the text was open

- **A move over a conflict** (parked by the standard's batch 2). A `Move` is staged as before, neither refused nor a resolution. Since a tombstone no longer settles anything, its tombstone now leaves the item's conflict standing, the diverging body pinned under the deleted item, where the write cleared the conflict and released the body before. Nothing is lost; whether a move should park as a remove does is the standard's to decide (tests/item_conflict_settle.rs `a_move_over_a_conflicted_item_keeps_the_diverging_body`).
- **A supersede over a conflict.** STORAGE §15.3 has the superseded item tombstoned "as a `remove` of it would": the remove states no resolution, so the whole `add` parks (`an_add_superseding_a_conflicted_item_parks_whole`). That follows from the text; no vector states it.
- **`if_match` over an item holding no body** is refused, for a remove as for an update, as before.
- **A payload stating no `v`, or one that is no integer, for a known action** is unknown, by the letter of §15.3 and as the standard's checker reads it, where the crate parked it.
- **A message carrying no summary** shares no `seq` and links no body: a row holding no summary agrees with no message, read both ways.

## Problems found in the standard's batch

- **Sync case 48** contradicts `held_elsewhere` and STORAGE §9, §11.1. Its store states no `mail_summary` row for Trash's `basic-1@example.org` and expects INBOX's bodiless row purged as held elsewhere. Under the new statement a bodiless message is held only by a row whose summary agrees with its own, a row holding none agreeing with none, so the row is retained. The case needs the summary of fixtures/mail-basic.eml stated on Trash's row (and the standard's checker skips the case for the same absence, which is why it passed there). It is named in tests/pending_vectors.txt with this reason, not worked around; tests/another_message.rs `a_bodiless_row_is_held_elsewhere_only_by_a_row_that_agrees` shows the same outcome on a store built by hand.
- **Annex A.1 does not say whether the last child of a multipart never closed is a part.** The summaries' walk reads none (a child no delimiter ends), mime-rs reads one, so the extraction may index a text the mark leaves unread, against SEARCH §6's "reading no part it leaves unread" if that child is one the walk leaves unread. Found by the differential test; pinned, not resolved.
- **Annex A.1 reads a field from its first occurrence "whichever column reads it"**, and SEARCH §6 names `Subject`, the address fields, `In-Reply-To` and `References`. `Content-Type`, `Content-Transfer-Encoding` and `Content-Disposition` are read by mime-rs from their last occurrence; the standard does not say the structure of the extraction's walk reads them from the first.

## Guards

Seen to fail on the behaviour before, by putting it back in the working tree for one run and restoring it: the hub clearing a conflict on every tombstone (the hub test and the move test), a link on the size alone (sync case 147), a remove meeting no refusal (143 and the cases after it), a `base` ignored (141), the last occurrence (the two search tests and the thread test). Seen to fail on the crate as it stood before the change: the two summary cases of an unusable `UID`, the payload suite, and the tests rewritten below, which stated the old rule. Not checked that way: the `seq` and held-elsewhere tests, whose old behaviour is the previous revision's statement text, which the vendored copy no longer holds.

- src/hub.rs `a_tombstone_settles_the_item_conflict_only_when_stated` and tests/item_conflict_settle.rs `a_move_over_a_conflicted_item_keeps_the_diverging_body`: both fail with the hub clearing the conflict on every tombstone.
- src/mutate.rs `a_remove_settles_only_the_conflict_it_names`, `a_remove_naming_the_conflict_adopts_the_diverging_body`, `a_remove_naming_the_item_conflict_states_the_settle`, `a_guarded_remove_refuses_a_replaced_body`, `a_flag_change_stating_its_base_keeps_what_landed_since`, `a_flag_change_stating_its_base_is_refused_over_an_unknown_set`; the two tests of a remove over a divergence they replace stated the old rule and failed once it moved.
- tests/queue.rs `a_queued_flag_change_stating_its_base_keeps_a_flag_landed_since`, `a_queued_flag_change_stating_its_base_parks_over_an_unknown_set`, `a_queued_remove_stating_its_body_parks_over_another`, `a_queued_remove_resolving_no_conflict_parks`, `an_unknown_payload_stays_pending_untouched_and_listed`, `a_handle_follows_its_row_until_the_stores_identity_moves`; tests/item_conflict_settle.rs `a_queued_remove_settles_only_the_conflict_it_resolves`, `an_add_superseding_a_conflicted_item_parks_whole`.
- tests/another_message.rs `a_message_shares_a_seq_across_mailboxes_only_with_a_row_it_agrees_with`, `a_lookup_answers_each_holder_with_its_witnesses`, `a_bodiless_row_is_held_elsewhere_only_by_a_row_that_agrees`.
- tests/search_pure.rs `the_extraction_reads_mail_as_the_summary_does` (it asserted `last` and passed before), `the_extraction_and_the_summary_read_one_field`; src/search/thread.rs `a_parent_is_read_from_the_first_occurrence`.
- tests/walk_agrees.rs `tree::a_built_message_has_the_attachments_its_tree_has` (512 cases, whole and cut), `tree::the_tree_and_the_walk_name_the_same_attachments`, `tree::a_child_no_delimiter_ends_is_a_part_of_the_tree_alone`. Its first run found the unclosed multipart above and a media type holding no `/`, read `text/plain` by A.1, which the test's own reading of the tree had wrong.
- src/codec.rs `a_payload_this_crate_does_not_know_is_kept_never_refused`; tests/payloads.rs judges the 66 cases by outcome; tests/store_rules.rs `a_reader_lists_a_row_of_a_later_payload_version_and_folds_nothing` (it asserted the row left out).
- src/sync.rs `a_create_holding_neither_body_nor_origin_is_reported_undeliverable`.
- Tests whose stores held mail rows with no summary and expected one `seq` across mailboxes (tests/queue.rs `copy_fills_the_target_and_move_also_empties_the_source`, tests/multi_account.rs `the_account_partitions_no_identifier`) now seed the summary a listing names a message by, which is what makes two rows one message.
- The engine's models state a person's remove as the resolution it is (tests/engine/property.rs `duo_remove`, tests/engine/conflict_retention.rs).

## Left

- The pin in pimdir's checks/io-pimdir.rev, which moves once this lands, and sync case 48 in the standard.
- Whether a `Move` meeting a conflict parks, and whether an unclosed multipart's last child is a part: the standard's.
- `acknowledge_action`, `replace_action` and `fail_action` take a bare id: an owner's writes on rows it just read. A performer following a row across a restore would want the handle there too.
- neverest, the Android app and the CLIs follow: `base`, `if_match` and `resolves` where they hold them, `waiting` as a list, the handle, `lookup_objects`' reply for a consumer servicing the seam itself, and `charset-tables` gone from their feature lists.
