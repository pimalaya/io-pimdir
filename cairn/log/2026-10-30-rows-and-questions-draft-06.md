---
cairn: log
change: rows-and-questions-draft-06
date: 2026-10-30
---

# Follow storage-draft-06 and search-draft-06

pimdir ffe31a4 split what a list reads: a summary holds the row, the index the questions (pimdir cairn/log/2026-10-30-rows-from-summaries-questions-from-the-index.md).

## What landed

- spec/ re-vendored from pimdir ffe31a4.
- src/summary/calendar.rs, src/summary.rs: `PimdirTaskSummary::created: Option<String>` and `contacts: Vec<PimdirAddress>`, `PimdirAddressRole::Contact` (`contact`), derived as Annex A.4 revision 2 says; src/client/write.rs writes `:created` and reads it back, files a `contact` row into `contacts`; src/cli/summary.rs prints `created`.
- src/client/schema.rs: `DERIVATION` 2; `derive` re-derives a revision 1 store on open (`rederive_task_capture`: `list_held_tasks` pages, `backfill_task_capture`, `replace_addresses`, `insert_address`, `stamp_item`, `set_derivation` with the last page).
- src/client/search/extract.rs, src/client/search.rs: `PimdirExtract::tel`, `adr`, `bday`, `photo`, `categories`, written by `insert_object` and `insert_object_category`; a `contact` address goes to no index field.
- src/search/query.rs, src/client/search/compile.rs: `PimdirQueryPredicate::HasProperty(PimdirQueryProperty)` (`Phone`, `Address`, `Birthday`, `Photo`) through `holding`; `category:` also through `categorised`.
- src/client/search.rs: `PimdirIndex::occurrences_between(&self, start: &str, end: &str) -> Result<Vec<PimdirSearchOccurrence>, PimdirError>`, `PimdirSearchOccurrence { collection, seq, start, end }`; `refresh` documents the owner's call after each write batch, which the store never makes itself.
- Tests: tests/task_capture.rs `a_reader_reads_a_revision_1_tasks_capture_as_null`, `the_owner_re_derives_a_revision_1_store_on_open` (fails with `DERIVATION` 1: the capture stays `NULL`); tests/search.rs runs the `occurrences` windows; tests/summaries.rs compares `created`; tests/store_version.rs expects derivation 2 after a released store's open; unit test `a_task_reads_its_contacts_on_annex_a4s_edges`.
- Capabilities moved: summaries ("A task carries its capture and its contacts", added), store ("The store records the derivation revision", modified: revision 2; "A store derived under revision 1 is re-derived on open", added), search ("A card's properties and categories are recorded at extraction", "An agenda reads a range's occurrences in order", added).

Verified on 2026-10-30 through `nix develop`: `cargo fmt`; `cargo clippy --all-features --all-targets -- -D warnings`; `cargo test --all-features`, 716 passed, 1 ignored; `cargo build --no-default-features` and `--no-default-features --features client`.

## Migration for the apps

- A `PimdirTaskSummary` built by hand gains `created` and `contacts`; a match on `PimdirAddressRole` or `PimdirQueryPredicate` gains `Contact` and `HasProperty`; a `PimdirExtract` built by hand gains the four flags and `categories`.
- An owner opening a revision 1 store re-derives every held task once, stamping each.
- An index.db an earlier build made is rebuilt on open (its `object` shape differs).

## Open

- `with:` reaching the `contact` role through the `people` field of a task's body (pimdir left it open; today only `item_address` holds it).
- The version bump: breaking changes to public types (`PimdirTaskSummary`, `PimdirAddressRole`, `PimdirQueryPredicate`, `PimdirExtract` fields), so 0.9.0 under 0.x.
