---
cairn: log
change: the-oracle-follows-search-draft-19
landed: 2026-11-25
---

# The oracle follows search-draft-19

Follows pimdir's change of the same name (search-draft-19): the SEARCH oracle of cairn/log/2026-11-25-the-oracle-answers-search.md brought to search-draft-18 and 19, finding G27 settled and lifted, G28 put to the owner.

## What landed

- src/client/search/compile.rs: a date term dates mail by its sort key and a calendar item by its occurrences alone, so one holding no body matches none (SEARCH §7 as search-draft-19 states it; finding G27). The sort key of a bodiless calendar placement dated it before.
- src/sql/search.rs, spec/queries/search/init_index_meta.sql: `search-draft-19`, `FORMAT_DRAFT` 19; an index of search-draft-18 is rebuilt.
- tests/search.rs: extraction.json `v` 2, a bodiless item stated by an `event_summary` row built through the owner as `mail_summary` and `file_summary` ones are; the version tests name draft 18 earlier and draft 20 later.
- spec/generator/src/query/grammar.rs: every refusal states its span (§8.4), a `dangling-operator` the bare `-`, else the first operator of a run missing its term, else the `and` or `or` missing a side; a phrase is a value, so `subject:""` parses, and `linked:<role>:` with no id is refused `invalid-value`; `reference_role` is the grammar's.
- spec/generator/src/query/date.rs: a relative amount closing a range spans its instant alone (`2w..2w` empty, `..2w` ending two weeks before now), where the oracle left such a range undecided.
- spec/generator/src/query.rs: a text term yielding no token and an empty date span match nothing, decided for every item; `thread:` joins the nodes of the whole store and answers the members of the named thread in every account holding one (it read each account apart, dropping `[work, 1]` from `thread:reply-1@example.org`); `Answer::matched`, what each hit's text terms matched on (`Matched::Body` where one of its placements whose body the index read matches, else `Matched::Summary` where a placement it did not read does, none for structured terms alone), decided where no text term stands negated and none is `person:`; a calendar placement whose body the index holds none of has no occurrence (`Readings::summary_dates` gone, G27).
- spec/generator/src/query/vectors.rs: refusal spans and `matched` compared; each store of extraction.json rendered and its queries held to the oracle too, under §7's default horizon around the file's `now`. All 353 cases of queries.json and the 83 queries of extraction.json: none disagrees.
- tests/generated/search.rs: each hit's `PimdirSearchHit::matched` compared with the oracle's where it decides it; G27 lifted from the known findings.

## Guards

- extraction.json "a calendar item holding no body": `kind:event date:2026-06-01` and `date:2026-06` fail on the old compile.rs (the item answered), checked by putting the old condition back.
- `query::vectors::tests::the_oracle_answers_the_search_vectors_alike_where_it_decides` failed on `subject:""`, the two `thread:reply-1@example.org` cases and `linked:sender:` before; `query::grammar::tests::a_dangling_run_answers_its_first_operator` pins the spans, `query::date::tests::a_point_covers_its_span_in_the_zone_stated` the range of relative amounts.
- `every_grammar_form_answers_what_the_oracle_decides_known_included` (ignored) now fails on G28 and G29 alone.

## Left

- G28, for the owner (pimdir cairn/log/2026-11-25-the-oracle-follows-search-draft-19.md): ical-rs voids a series every rule of which ends on a day before its `DTSTART` (src/recur/set.rs `is_cut_before_start`), where RFC 5545 §3.8.5.3 has `DTSTART` define the first instance. Until ruled, the oracle leaves it undecided (`Readings::start_counts`).
- spec/REVISION still names an earlier pimdir commit: tests/spec_drift.rs fails on the vendored files until the pimdir commit exists.
- spec/generator/README.md, the other lane's, still describes the oracle's grammar by its codes alone.

## Spec

[search](../spec/search.md): search-draft-19 throughout; `date:` compares a calendar item's occurrences and nothing else; extraction.json at `v` 2. [spec-fidelity](../spec/spec-fidelity.md): the README's conformance line names search-draft-19.
