---
cairn: log
change: the-crate-follows-storage-draft-09
date: 2026-11-05
---

# The crate follows storage-draft-09, and a delta loads what owes a push

spec/ is pimdir at 1d34d2f: storage-draft-09, sync-draft-08 and search-draft-09 (`sql::FORMAT_VERSION`, `sync::SYNC_VERSION`, `sql::search::FORMAT_VERSION` and `FORMAT_DRAFT`). A store draft-08 wrote is refused by every profile, never migrated.

What moved:

- **The system flags are bits of the row** (store.md, flags requirement): `insert_item` and `update_item` bind `:marks`, `insert_binding` and `update_binding` `:base_marks`, from `codec::marks_to_int` (`codec::MARKS` in bit order, `codec::mark_of`); `write_flags` fills the `flags` slot with the other flags alone, `\seen` among them. Reads answer one joined set as before, so `PimdirFlags` and the projection are unchanged. Guarded by tests/owing_load.rs `a_system_flag_is_a_bit_and_its_lowercase_a_row` and every vector under vectors/sync.
- **The crate's own SQL reaches a child by the item's id**: `DANGLING_BINDINGS` (naming the lost item `#<id>`, its link id gone with it) and `DELETE_DANGLING_BINDINGS` in src/sql.rs, `LOAD_HIT_PLACEMENTS` (the bits rendered) and `MESSAGE_IN_REPLY_TO` in src/sql/search.rs, and the search compiler's summary, category and address joins in src/client/search/compile.rs. Guarded by tests/spec_drift.rs, which prepares every statement, and the search suites.
- **A delta loads what owes a push** (seam.md, a-delta-loads-what-it-names): `PimdirLoadSelection::Owing(handles)`, answered by `PimdirSourceStore::load` from `list_owing_bindings`, `list_lacking_items` and the handles named; `PimdirSync` asks it of a delta's listed and vanished handles, then the `Links` of its hints, as a page before the last does. A round's last page and a rekey keep `All`: what they find absent and whether they park are counted over every based binding of the scope (pimdir SYNC §5, §8). Guarded by src/sync.rs `a_delta_loads_what_owes_a_push_never_the_collection`, which fails with the delta loading `All`, tests/owing_load.rs `a_delta_loads_what_it_names_and_what_owes_a_push`, and every vector under vectors/sync passing over the narrower load.

The tests' own SQL (the vector harness's seeding and readback, gc, rederivation, roundtrip, refcount_property, search) follows the new keys; refcount_property's `is_conflicted` swallowed its SQL error, so its interesting-states assertion was what caught it.

Left open: the last page's `Owing` load (a count of the scope's based bindings by statement), and the 100k and 1M bench the change folder lists.
