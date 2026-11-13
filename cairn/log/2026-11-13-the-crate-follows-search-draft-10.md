---
cairn: log
change: audit-2026-11-12
landed: 2026-11-13
---

# The crate follows draft 10, the search batch

spec/ is pimdir at 3fabace, the fourth and last batch of storage-draft-10, sync-draft-10 and search-draft-10: the index schema gained `index_meta.extraction` and `extractor` and the partial index `object_not_ok`, one statement is new (`set_extraction`), three changed their signature (`coverage`, `init_index_meta`, `load_index_meta`), vectors/search/store.json moved to `v` 2 and queries.json to `v` 4 with 33 new cases (212), and one fixture is new (mail-lost.eml). STORAGE.md and SYNC.md did not move, so no store is refused that opened before; an index written under the earlier text of draft-10 is rebuilt by its schema text. The copy was taken from the commit (`git archive 3fabace`), never from the standard's working tree. tests/pending_vectors.txt is empty: the 212 cases pass, none worked around.

The audit's lines: S2, S4, S6 and V7 followed, S1 and S3 checked against the text as now written and S1's last gap closed, D5's search half verified, and C5's completeness hoisted into the crate.

## What followed

- **The revisions of an extraction** (SEARCH §2, §6; V7). `search::extract::EXTRACTION` is 1 and `EXTRACTOR` is `io-pimdir 1`, the first value any index records: an index built before lacks the column and is rebuilt by its schema text, so the reduction as it stands after this change, tags separating words (3c10177) and hidden elements told by their first word (below), is revision 1. The constant's comment says when to bump it. `init_index_meta` binds both. `PimdirIndex::open`, and `refresh` under index.lock, rebuild an index recording another of either (`PimdirIndexHeld::Extracted`), the simplest conforming way: an in-place path needs a statement of the crate's own walking the held bodies and a new arm of the refresh, for nothing a rebuild does not give.
- **The open reads the schema text first.** `held` ran `load_index_meta` before it compared the schema, and the statement now names two columns an index of the earlier text lacks, so an open would have failed on "no such column" where it must rebuild. The comparison moved before the read.
- **Coverage in four counts** (SEARCH §8.2; S2). `PimdirSearchLocal::kinds` holds per kind a `PimdirSearchUnread` (`bodiless`, `missing`, `unparseable`, `encrypted`), read by its five columns off the canonical `coverage`; `PimdirSearchLocal::bodiless` is gone, no alias. `PimdirReader::search_coverage` fills `bodiless` alone.
- **The query's collections.** A parenthesised group parses to `PimdirQuery::Group`, and `PimdirQuery::conjuncts` answers the terms standing as conjuncts of the whole query. Before, a group of one term was indistinguishable from the term, so `(collection:a) x` ruled collections out where the text has it rule none.
- **A complete answer** (SEARCH §8.2; S2, C5). `PimdirQuery::reads` derives what the terms read (`PimdirSearchReads`), carried in the coverage as `local.reads`, and `PimdirSearchCoverage::is_complete` is the text's definition, the lag included: `behind` was no part of it, and the three downstream CLIs added it themselves.
- **The zone of a query against the zone of its index** (SEARCH §8.1; S6). `PimdirSearchCoverage::zone_differs`, compared as names, for a query holding a date term whose collections hold a calendar one.
- **The grammar** (SEARCH §8.1, §8.3, §8.5; S4). `relative` is at most six digits and `stamp` at most eighteen (`RELATIVE_DIGITS_MAX`, `STAMP_DIGITS_MAX`). A range's start before `0000-01-01T00:00:00Z` and its end past `9999-12-31T23:59:59Z` are open (`compile::bounds`); the clamp to year 0 is gone. U+0000 in a text term compiles to a space.
- **The CLI.** `coverage.local.kinds`, `coverage.local.reads` and `coverage.zoneDiffers` in JSON and in the JSON Schemas, which are derived; the text prints the counts the query's terms read, and the zone under a complete coverage too.
- **The vector runner.** store.json at `v` 2: an item stating `unreadable` is built as any other and its blob removed before the index is built. queries.json at `v` 4: the file's `coverage` against the index over every collection, a case's `coverage` and `complete`, `zone_differs` for every case, and `index_zone`, the cases grouped by the zone their index states and asked in the zone they state. Every case also compares the four counts with those derived from the store file, and a lag of zero.

## S1, S3 and D5 against the text

- **S3** (SEARCH §4, §7). The text now says what 3c10177 does: a re-expansion that cannot read a body keeps the occurrences and the cut and records the placement; every pass expands again each calendar placement `load_missing_items` answers, its body indexed or not; a placement leaves `missing` only once its body was read for the indexing and, for a calendar item, the expansion. No code moved. The standard asked for the test of a roll: `a_roll_keeps_the_occurrences_of_an_unread_series` is new beside the zone one.
- **S1** (SEARCH §6). The reduction separated tokens at tags since 3c10177. It skipped `head`, `style`, `script` and `template` only as tags of one word, so `<style type="text/css">` put a message's CSS in the index, which 2026-11-12's log left as not S1's. It is now told by the tag's first word, as the inline test already read it.
- **D5** (SEARCH §9). The five cases over item 33 pass on 02c9191's code, unchanged.

## Behaviour chosen where the text was open

- **A bodiless item counts against every query**, as the text says and the standard parked for its owner. A headers-only mailbox therefore never answers complete, whatever the query.
- **A group of one term.** By the letter, `(collection:a) x` is covered over every collection and `collection:a x` over one, though they answer the same hits. Followed as written, at the price of a new AST variant.
- **Which terms are text terms.** Every `Text` predicate reads every kind's body, a value holding `@` included, since the crate matches it in the index. An address field's value holding `@` is a seek of the store and reads none. `person:` reads every kind's whatever its value, the crate matching the card's title in the index.
- **A query client over another extractor's index** answers from it and cuts its snippets from its own chunks, as SEARCH §6 asks of a client whose extraction is not the indexer's; it refused nothing before, there being no way to tell. One under another `extraction` number is answered the same way: the text gives a query client no refusal on it.
- **The crate runs no `set_extraction`**: a rebuild's `init_index_meta` records the revisions, in the transaction creating the index.
- **A range's end before year 0** (`..999999d`) is the first instant of year 0, so the range holds nothing. The text opens a start there and says nothing of an end.
- **Zones are compared as names**, so `Etc/UTC` against `UTC` differs, by the letter.
- **The text output under a complete coverage** prints the differing zone alone; under a partial one, a count the query's terms do not read is left out.

## Problems found in the standard's batch

No vector contradicts its prose, and none is on tests/pending_vectors.txt. In the prose:

- **§8.2, "inside a group".** A redundant pair of parentheses changes an answer's coverage and may change whether it is complete, the hits being the same. checks/vectors.py does not read a query holding a group, so no case pins it either way.
- **§8.2 and §8.3, a text field's value holding `@`.** §8.3 calls it "an address, not text" for every text field, `subject:` and `body:` included, and §8.5 makes a text term only of a value holding none; §8.2's "a text term reads every kind's body" then leaves `subject:a@b` reading nothing, where every implementation matches it in the index.
- **§8.2, `person:`.** A term the index answers from a card's extracted title and the store's addresses; the list of what each term reads names it only as a text term.
- **§8.1, an end before year 0** is unstated.
- **§2, a query client and `extraction`.** The text says what a query client does with another `extractor`, nothing of another `extraction`.
- **§2, the rebuild and `set_extraction`.** "record its own (`set_extraction`) in the transaction completing it; it MAY rebuild the index instead": a rebuild records through `init_index_meta`, which the sentence does not say.

## Guards

Seen to fail with the behaviour before put back in the working tree for one run, the files then restored from copies kept beside them (never through git):

- the hidden elements told as one-word tags: src/search/html.rs `a_hidden_element_is_skipped_whatever_its_attributes`;
- no comparison of the revisions: tests/search.rs `an_index_under_another_extraction_is_rebuilt_and_every_body_indexed_again`;
- `load_index_meta` read before the schema text: `an_index_of_the_earlier_draft_10_text_is_rebuilt`;
- U+0000 left in the expression: the two vector cases ("unterminated string") and src/client/search/compile.rs `a_nul_in_a_text_term_is_written_as_a_space`;
- no open side: the vector cases `date:today..9999` and `kind:mail date:2026-10..9999-12-31`, and `a_bound_no_four_digit_year_states_is_open`;
- unbounded digits: the vector cases `date:1000000d` and `changed:1000000000000000000`, and src/search/query.rs `dates_and_stamps_read_their_forms`;
- a group parsed to its operand: `a_term_in_a_group_is_no_conjunct` and tests/search.rs `a_coverage_counts_what_the_index_could_not_read`;
- completeness reading no term (every count counting) and no lag: seven of the vector cases stating `complete`, src/search/coverage.rs `a_body_out_of_sight_counts_only_against_a_term_that_reads_it` and `a_lag_is_never_complete_and_another_zone_is`, tests/search_cli.rs `a_search_prints_the_four_counts_and_whether_it_is_complete`.

New, with no earlier code to fail on: `a_term_reads_the_bodies_of_its_kinds`, the unparseable count of `a_coverage_counts_what_the_index_could_not_read` (the vectors hold none, a parser's refusals being the implementation's own), src/cli/search.rs `a_coverage_prints_the_counts_its_query_reads`, and `a_roll_keeps_the_occurrences_of_an_unread_series`, which states S3's rule over a roll and passes on 3c10177's code. tests/search.rs `memory::a_consumer_of_its_own_shape_folds_the_canonical_index` now counts the vectors' unreadable item among its missing records and names it once the other is read.

## Left

- The pin in pimdir's checks/io-pimdir.rev, which moves once this lands.
- The standard's owner questions above, and whether a bodiless item counts against every query.
- An in-place re-extraction, should a rebuild ever cost too much: `sql::search::SET_EXTRACTION` is generated and unused.
- `<br clear="all">` and any `br` carrying attributes is still no line break (mail-parser's one-word rule), only a separator.
- The Android app's FTS4 index owes the same rules but the canonical statements; src/search/ stays shape-neutral: `EXTRACTION`, `EXTRACTOR`, `PimdirQuery::conjuncts` and `reads`, and `is_complete` are I/O-free, and the refresh coroutine reads no revision.
- The downstream CLIs and the Android app follow: `local.kinds` where they read `local.bodiless`, `is_complete()` alone where they wrote `is_complete() && behind == 0`, the `Group` arm where they match on `PimdirQuery`, `zone_differs` in calendula's JSON.
