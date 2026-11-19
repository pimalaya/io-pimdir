---
cairn: log
change: the-store-rules-the-clis-restate
landed: 2026-11-19
---

# The store rules the CLIs restate

On the owner's ruling of 2026-11-19 (pimdir audit-2026-11-12 C5, every recommendation taken), the rules himalaya, cardamum and calendula each restated in their src/pimdir trees are now the crate's, additive:

- `collection::media_type(kind)`: the bare media type, which `role_of_kind`, `summary::derive`, the index extraction, the reader's summary page and the write path's kind tests now call instead of their own split.
- `PimdirReader::accounts_of(kind)`: the accounts holding collections of a media type (`None` for every kind), sorted, deduplicated, an undeclared collection counting for no media type. The ruling for the CLIs' choice: a configured account is accepted on a store holding nothing yet, and an empty kind is no application's.
- `PimdirQuery::scoped(kind, account, collection)`: the scope joined to the top-level `and`, where SEARCH §8.2 reads coverage.
- `PimdirSearchCoverage::unread_counted()`: the counts out of sight that count against the answer; `is_complete` reads it, so the rule is written once.
- `PimdirReader::overlay_quiet(collection)`: from the overlay's own fold, whether or not the reader overlays.

Capabilities moved: store ("Collections declare a media type", ADDED "The accounts of a media type are read in one place", ADDED "A reader says whether the queue changes a collection's read"), search (ADDED "A frontend scopes a query and counts its gaps through the crate").

Tests: tests/multi_account.rs `the_accounts_of_a_kind_leave_an_undeclared_collection_out`, tests/overlay.rs `a_collection_is_quiet_until_an_action_changes_what_its_read_shows`, src/search/query.rs `a_scope_stands_as_conjuncts_of_the_whole_query`, src/search/coverage.rs `only_the_counts_a_term_reads_are_counted`. New public surface, so none fails against the old code but by not compiling; the existing coverage tests and the search vectors pass unchanged over `is_complete` rewritten on `unread_counted`, which is the check that the rule did not move.

The three CLIs drop their copies in their own sessions once a release carries the helpers; what each deletes is listed in the change's proposal.md.
