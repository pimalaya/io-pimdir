---
cairn: log
change: audit-2026-11-12
landed: 2026-11-12
---

# The crate follows draft 10, the schema and the statements

spec/ is pimdir at 31da94c, the first batch of storage-draft-10, sync-draft-10 and search-draft-10 (`sql::FORMAT_VERSION`, `sync::SYNC_VERSION`, `sql::search::FORMAT_VERSION` and `FORMAT_DRAFT`, README.md). Its migrations/ and queries/ moved; its vectors/ and payloads/ are those of 317768b byte for byte, the later batches owning them, and a later pass re-vendors those. A store draft-09 wrote is refused by every profile and its index rebuilt, never migrated. The copy was taken from the commit (`git archive 31da94c`), never from the standard's working tree, which was moving.

## What followed

- **The draft names.** The three constants, README.md's conformance line and note, the spec's scenarios. The tests naming a later draft name draft-11 (tests/cli_owner.rs, tests/open_current.rs, tests/search.rs, tests/store_version.rs, src/version.rs), and draft-09 joins the refused and the rebuilt ones; the older fixtures stay as refused stores.
- **Eviction.** `PimdirStore::collect_before` is `evict_before`, `PimdirCollectReport` is `PimdirEvictReport`, `sql::COLLECT_BEFORE` is `sql::EVICT_BEFORE`, no alias. The CLI has no verb for it.
- **Statements cut.** `sql::COMPONENT_OF` and `sql::search::MATCH_OBJECTS_RANKED` went with their files: nothing here ran either, so no text of the crate's own replaces them. The order by relevance stays the crate's own compiled statement over the rank `match_objects` and `match_summaries` answer.
- **`hit`.** The canonical statement, re-signed `(:seq, :account)` and answering nine columns, is what `PimdirIndex` reads a hit's placements with; `sql::search::LOAD_HIT_PLACEMENTS`, the same text under the crate's name, is deleted.
- **Names.** `:before` in `purge_retained_before`, `release_before` and `release_bases_before` (and in the crate's own `COUNT_RETAINED_BEFORE`), `:hash` in `held_elsewhere`, `held_mail_summary` and `origin_for_link`, `:message_id` in `insert_in_reply_to`; the crate's own `MESSAGE_IN_REPLY_TO` reads `r.message_id`. The parameter sets of the two revisions were compared statement by statement: these, `hit` and `evict_before` are the only ones that moved, so no binding is left unnamed (SQLite binds `NULL` to a parameter nothing names, and rusqlite refuses a name the statement lacks).
- **A receipt outlives its collection.** `PimdirActionStatus::Applied::collection` is an `Option`; `pimdir queue status` prints "a collection deleted since", its JSON `collection` `null` (the output's schema already stated it nullable, so `pimdir json-schema` is unchanged).
- **The incarnation.** `raise_incarnation` documents the random stride; the existing test stated no exact value, and a second one holds two restores of one backup apart.
- **`NULL` is the first page.** `list_items`, `list_items_page_asc`, `list_summaries` and `list_retained` bind `None` where they bound `''` and `0`; the special case of the descending first page is gone with it.
- **`purges` on a collection's delete.** The index reconciled a collection's delete through its items' purges and still does; an empty collection's delete now reconciles too, removing nothing.
- **A roll over journals.** `items_to_reexpand` answers every live journal when the horizon moved; the refresh expands what the statement names whatever its component, so a recurring journal now gains the new window's occurrences. Before, it kept those of the horizon it was indexed under.
- **queries/signatures.json** is vendored with queries/ (tests/spec_drift.rs compares the directory both ways) and is no statement: build.rs reads the profile directories alone, as its comment now says.

## The shapes the schema holds

Every write path was read against the new `CHECK`s, and the whole suite run on the new schema before any test was touched: no production path and no test's seeding tripped one. What moved is what a caller could state in another shape:

- **A base has one witness.** The writes already bound the three value columns from the base alone. `binding_from_row` read any of the four as a witness; it reads `base_present` alone.
- **A scope bound.** Nothing checked the shape of a `PimdirScope` a caller states, which was stored and compared as text. `PimdirScope::malformed_bound` names a bound not written `YYYY-MM-DDTHH:MM:SSZ`, and `prepare_sync`, `sync` and `write` (`OpenRound`, `CloseRound`, `SetCoverage`) refuse it as `PimdirError::ScopeBound` (`scope-invalid`) before anything is listed or written.
- **A size.** A summary size was bound `as i64`, so a figure above `i64::MAX` was stored negative; under the `CHECK` it would have failed the page naming it. It is written unknown.
- **The tests' own SQL.** tests/vectors_sync.rs and tests/search.rs read and seed `mail_in_reply_to.message_id`. No seeding wrote a row the schema refuses.

## Guards

- tests/search.rs `a_rolled_horizon_re_expands_a_recurring_journal` fails on the draft-09 text of `items_to_reexpand` (the vendored file swapped back in a run, then restored from the archive); `an_empty_collection_s_delete_is_reconciled`.
- tests/store_rules.rs `a_stated_size_no_integer_holds_is_unknown` fails with the wrapping cast, on `CHECK constraint failed: size >= 0`; `a_domain_is_compared_never_a_pattern`, the crate holding no test of `domain_placements` before.
- tests/scoped_sync.rs `a_scope_bound_of_another_shape_is_refused_before_any_write`.
- tests/receipts.rs `a_receipt_outlives_its_collection_and_names_none`.
- tests/store_version.rs `two_restores_of_one_backup_answer_two_incarnations`.
- src/cli/queue.rs `an_applied_action_says_when_its_collection_is_gone`.

## Left

- What a caller alone states and the schema now refuses (an empty source, account, collection id, handle or link id; a receipt `seq` below 1 through `acknowledge_action`) reaches the caller as `sql`, a constraint failure, the store unmoved. Whether each deserves a code of its own is open; a connector listing an empty handle fails its page there.
- The crate does not read queries/signatures.json. A suite preparing every vendored statement and comparing its parameters and columns with the listing would repeat the standard's own check under this crate's SQLite.
- The vectors and payloads of the later batches, and the pin in pimdir's checks/io-pimdir.rev, which moves once this lands.
