---
cairn: log
change: the-oracle-answers-search
landed: 2026-11-25
---

# The oracle answers search

Before this, nothing asked a SEARCH query of a generated store: tests/generated/index.rs, crash.rs and scale.rs compare a refreshed index with a rebuilt one table by table, and the generator's oracle/search.rs answers the store's own `LIKE` statements, not SEARCH §8. This closes pimdir's freeze-plan lines "The oracle: search answers" (section 2), "Query vectors: every form of the grammar, run over generator stores" and "The generator's oracle answers every search the vectors ask" (section 5b).

## The generator

- spec/generator/src/query.rs, with query/grammar.rs, query/text.rs, query/date.rs: SEARCH §8 read from SEARCH.md's text alone, never from src/, over the model. §8.3's grammar with §8.4's codes (spans left to the vectors); §5's fold and `unicode61` split over ASCII, Latin-1, Latin Extended-A and Greek, any other character no answer; §8.5's match, a phrase found only across two values of a field no answer; §8.1's spans in `UTC` and `Europe/Paris`; §7's occurrences of the series the renderers write; §9's threads under the owner's ruling of 2026-11-25 (the value of `thread:` is the link id of any node, member or phantom, in every account holding it), a member bearing its link id or its link id and its `Message-ID`, both read; the five orders, every reading of the ties and of the hit's key §8.2 leaves open (seq and account either way, the key over every placement or the query's collections, an empty sort key dated or not); the scope and the coverage. Every term is three-valued per hit, `query::Readings` naming the two sentences found to read two ways (G27, G28), undecided by default.
- spec/generator/src/body.rs: `Filed::words`, what §6 extracts from a body the generator rendered, read off the values it wrote (README.md, "No parser"); `Model::words` keeps them by object name, the queue's `add` and `update` bodies included. A fixture's is never known.
- spec/generator/src/upkeep.rs: purge, sort key, name, rename, release and eviction below a date, erase, collection delete and the collector on the model, read from STORAGE §11 and §14, each `purges` counted as §4.5 says.
- spec/generator/src/query/vectors.rs: vectors/search/store.json rendered as a model (a fixture summaries.json pins no row for held opaque: only what store.json states beside it decides a term), each case of queries.json held to what the oracle decides. Of the 302 cases, 123 agree wholly, 126 on what the oracle decides, 53 are left to fixture bodies or a threaded listing, none disagrees. Three answers were the oracle's to fix on the way: an address value takes no wildcard, `~` alone is a reserved form, a parenthesis pass answers its leftmost fault.
- spec/generator/src/oracle.rs: an applied row's receipt states a `NULL` collection once that collection is deleted, as the receipt table's `ON DELETE SET NULL` leaves it. spec/generator/src/store.rs: the derivation check reads src/summary.rs, where `DERIVATION` now stands.

## tests/generated/search.rs

Per seed of common.rs, the structure profile's store (tamed as index.rs tames it) is taken through index.rs's run of twenty owner writes and the model through the same writes by `upkeep`, the index refreshed after each in batches of three. reader::compare first holds the model to the store; then about 176 queries drawn from the model (every field of §8's table, every value of the closed fields, ids, `Message-ID`s, addresses, domains, words, phrases, wildcards, every `date-value` shape, `or`, `and`, `not`, `-`, groups, each text form also narrowed to one generated account) are asked of io-pimdir's `PimdirIndex` through four read-only handles, in `newest-first` and one other order in turn: the hits against what the oracle decides, the list in every order but `relevance` where the oracle settles it, the pages walked at a limit of 1 to 3 against the whole answer, the count, a count's coverage against the search's, the lag, the four counts per kind, `temporal`, `horizon_left`, `cut`, `zone_differs` and `complete`. Two deliberate breaks of the oracle (`is:unread` read as `is:read`, `newest-first` read ascending) fail it with 622 and 1413 differences.

Findings, each with its constant and `every_grammar_form_answers_what_the_oracle_decides_known_included` (ignored) failing while it stands:

- G27 (spec, minor, reads two ways): a calendar placement holding no body is dated by io-pimdir at its sort key (seed 7, lab/calendar seq 115, the hostile-zone fixture index.rs leaves bodiless, answers `kind:event date:2026-06`); SEARCH §7 expands bodies alone and §8.1 compares occurrences.
- G28 (spec, minor, reads two ways): a weekly series whose `UNTIL` falls before its `DTSTART` (seed 7, work/calendar seq 49: `DTSTART:20261122T143216Z`, `RRULE:FREQ=WEEKLY;UNTIL=20260512T181043Z`) yields no occurrence in io-pimdir, so `kind:event date:2026-11-22` misses it; RFC 5545 counts `DTSTART` as the first occurrence of a rule bounded by `COUNT` and is silent for `UNTIL`, SEARCH §7 too.
- G29 (outside search; spec, minor, reads two ways): a release below a date clears the binding's `shared_object` as well as its `base_object` in io-pimdir (seed 7, step 7, `release_before(["work/INBOX"], None)`), so the collector then frees the body and counts it; STORAGE §11.4 names `base_object` alone and §5 has `shared_object` outlive the item's body.

`every_grammar_form_in_every_order_answers_what_the_oracle_decides` (ignored) asks every query in all five orders and walks each at 1, 2 and 3.

No production code moved. Capabilities moved: none (tests and the generator only).
