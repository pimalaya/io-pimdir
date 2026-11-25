---
cairn: log
change: three-owner-rulings
landed: 2026-11-25
---

# Three owner rulings

Follows pimdir's change of the same name (storage-draft-19, search-draft-20, sync-draft-16 unchanged): the owner's rulings of 2026-11-25 on G28, G29 and a mutable revival whose member carries another body. This crate already answered all three as ruled; the generator follows, and tests/generated holds no known finding.

## What landed

- src/sql.rs, spec/queries/storage/owner/init_store_meta.sql: `storage-draft-19`; a store of storage-draft-18 is refused. src/sql/search.rs, spec/queries/search/init_index_meta.sql: `search-draft-20`, `FORMAT_DRAFT` 20; an index of search-draft-19 is rebuilt. src/cli/store.rs's sample, README.md, and the version tests (src/version.rs, tests/store_version.rs, tests/open_current.rs, tests/cli_owner.rs, tests/search.rs) moved to N-1 and N+1.
- spec/generator/src/query.rs: a series whose `UNTIL` states a date before the one its `DTSTART` states has no occurrence, the date compared as ical-rs compares it, not the instant (SEARCH §7, G28); `Readings` and `Corpus::readings` gone with the one reading they held (`start_counts`), spec/generator/src/query/vectors.rs following.
- spec/generator/src/upkeep.rs: `release_before` lets a binding's `shared_object` naming the released body go beside its `base_object` (STORAGE §5, §11.4, G29), as release_bases_before.sql does.
- spec/generator/src/engine.rs: `revive` keeps the retained body apart only when the member carries none; one carrying another body is adopted and the old one released (STORAGE §11.1).
- tests/generated/search.rs: G28, G29, the `released` classifier and the ignored `every_grammar_form_answers_what_the_oracle_decides_known_included` twin gone, every difference failing the suite. `queries` gains the forms of SEARCH §8.3 it did not ask (pimdir's plan 5b check): a field value's wildcard, an instant's numeric offset, the `min`, `h`, `w`, `mo` and `y` units, a single and a closed `changed:` stamp, `AND` and `NOT` in capitals, a negated phrase, juxtaposition after `)` and after a phrase, TAB, CR and LF as separators; every one answers as the oracle decides.

## Guards

- G28: pimdir's extraction.json case "a series ending before it starts" (fixtures event-cut-before-start.ics and event-until-same-day.ics) passes here and fails on this crate built against an ical-rs 0.7.3 whose `is_cut_before_start` answers false (`kind:event date:2026-09-14` answered) or compares instants (`kind:event date:2026-09-16` missed), each run with `--config patch.crates-io.ical-rs.path` on a scratch copy. tests/generated/search.rs asks the oracle, which leaves no such series undecided now, the generated `DTSTART:20261122T143216Z` with `UNTIL=20260512T181043Z` among them.
- G29: tests/generated/search.rs with the classifier gone failed on the old upkeep ("the model does not follow the write" on `storage/read/list_item_bindings`, from each seed's release step on) and passes now. No vector: pimdir's sync vectors have no owner release verb; tests/invariants_retention.rs `l1232_a_release_clears_only_the_released_shared_object` and `l1237_released_bodies_fall_to_the_collector_alone` pin this crate's side, unchanged.
- The revival: pimdir vector 233 passes here and fails with `revive_item` keeping a retained body that differs from the carried one ("events differ"); spec/generator/src/history.rs `a_retained_card_revived_with_another_body_adopts_it_and_keeps_none` failed on the old `revive` (two item rows) and passes now.

## Spec

[store](../spec/store.md): the storage-draft-19 names; *A mutable revive waits at Meta* gains the scenario of a card restored carrying another body; the release requirement names the shared body released with the base. [search](../spec/search.md): search-draft-20 throughout; occurrences state the series ending before it starts. [spec-fidelity](../spec/spec-fidelity.md): the README's conformance line, stale at storage-draft-17 and sync-draft-15, now storage-draft-19, sync-draft-16 and search-draft-20. [cli](../spec/cli.md): the later draft a refused store names is storage-draft-20.

## Left

- spec/REVISION names an earlier pimdir commit: tests/spec_drift.rs fails on the vendored files until the pimdir commit of this change exists and REVISION names it.
- spec/generator/README.md, the other lane's, still describes the eleventh part's revive by its bodiless case alone.
