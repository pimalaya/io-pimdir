---
cairn: log
change: a-smaller-pimdir
landed: 2026-11-22
---

# A smaller pimdir: the crate owns its SQL, storage-draft-17 and search-draft-17

The crate follows pimdir's redesign of the night of 2026-11-22 (pimdir cairn/changes/a-smaller-pimdir, its decisions.md and journal.md; pimdir's own entry is its cairn/log/2026-11-22-a-smaller-pimdir.md). Commits, unsigned and unpushed: 286c50f (pimdir 4250177), 4a0ece9 (0f5e542), 3189d3a (61b39c4), 3a1f3eb (cb23af5); spec/REVISION names cb23af5. Capabilities folded: store, search, spec-fidelity, summaries, cli, sync (a pointer to the retired GUIDE.md).

## What moved

- **The statements, the index schema and the generator are the crate's** (286c50f). pimdir binds none of them any more: spec/queries/, spec/migrations/search/ and spec/generator/ stay where they were (decisions.md, 21:14: build.rs, the generator's tree lookup and twenty test files read those paths) and are edited here. tests/spec_drift.rs compares only spec/migrations/storage/, spec/vectors/ and spec/payloads/ with pimdir (`VENDORED`); the name and prepare checks still run over every statement file. The refresh vectors pin answers alone (refresh.json `v` 6), and tests/search.rs `every_refresh_vector_answers` now holds, after every step, that the refreshed index and a rebuilt one hold as many `object` rows and the same `split` rows.
- **Flags as columns** (4a0ece9). The owner writes every flag but the system ones to `items.keywords` and `bindings.base_keywords`, sorted bytewise and joined by one space (`codec::keywords_to_text`, `write::keywords`), in the row's own insert or update; `write_flags` and the statements `add_flags`, `retain_flags`, `add_base_flags` and `retain_base_flags` go. Every reader builds the flag text from `marks` and `keywords`, so no public type moves.
- **One summary table and one value table** (3189d3a). The owner writes one `summary` row whatever the component (`upsert_summary`) and the lists as `summary_value` rows (`replace_values`, `insert_value`); the readers keep their statement names and answered columns through aliases (`title AS subject`, `uid AS message_id`, `card_kind AS kind`); the search compiler and index.hits read `summary` and `summary_value`. 34 storage statements go and 9 come (213 to 188). A calendar resource or an undeclared collection's body changing component replaces its row in place (decisions.md, 22:02).
- **Lighter guards and index** (3a1f3eb). `delete_collection` asks `collection_holds_actions` before the delete, the schema's `ON DELETE RESTRICT` on `queue.collection` refusing it too; `deliver_binding`, `link_senders_of_by_link`, `list_garbage_objects` and `set_store_version`, which nothing ran, go. The index drops its `chunk` and `thread` tables, which nothing read (a snippet cuts its chunks from the body, a thread's span is read from the store), and `chunks_of`, `insert_chunk`, `upsert_thread`, `delete_thread` and `set_extraction`; `PimdirIndexOp::PutThread` and `DropThread` stay for a consumer of another shape and are skipped here. The index's format is `search-draft-17`, an earlier one rebuilt.

`sql::FORMAT_VERSION` is `storage-draft-17`: a store of draft-16 is refused as an earlier engine's. Statement files 280 to 243 (storage 217 to 185, search 63 to 58), index tables 13 to 11.

## Gates (journal.md)

- Baseline at 00727ba: 1,089 passed, 0 failed, 8 ignored.
- 4a0ece9: 1,090 passed (the keyword test added), 0 failed once REVISION moved (spec_drift 7 of 7), 8 ignored; clippy clean; `cargo test --no-default-features --lib` 436; pimdir's flake checks 5 of 5. The plans of the mail page, the unread page, the counts and the owing load unchanged.
- 3189d3a: 1,165 passed, 0 failed (spec_drift 7 of 7), 8 ignored; 200 of 200 sync vectors, the search vectors unchanged; clippy, fmt, no_std 436; pimdir's flake checks 5 of 5 on 15 tables. Plans unchanged but for index names; `evict_before` scans `summary` where it scanned `mail_summary`.
- 286c50f went in without its own gate, the step-3 agent editing the tree; 4a0ece9's gate covers both. The journal records no gate for 3a1f3eb.

## The 76 ported tests

pimdir's checks/invariants.sh left the standard with the statements it ran by name. Its 564 rules were mapped (pimdir's invariants.tsv): 368 already held by a test or a vector here, 196 not. 76 tests port the data-safety ones (tests/invariants_schema.rs 29, invariants_retention.rs 15, invariants_seq.rs 5, invariants_index.rs 5, invariants_references.rs 5, invariants_flags.rs 4, invariants_refcounts.rs 4, invariants_marks.rs 3, invariants_queue.rs 3, invariants_supersedes.rs 3, their helpers in tests/invariants_common/), each named by the invariants.tsv lines it holds and each shown to fail by breaking its rule on a scratch copy. 48 plan rules and 17 migration rehearsals were skipped by design and about 35 rules stay owed. Found on the way, not fixed: `PimdirReader::get_item` answers `invitation: None` for a mail whose summary holds one.

## Owed

- The keyset-plan checks of the crate's own statements on SQLite 3.18, which pimdir's checks/floor.sh no longer runs and the Android app needs (decisions.md, 21:23).
- No consumer is ported: the four CLIs and the Android app stay on 0.19.1 or their pins; pimdir's consumers.md lists what each names that moved.
