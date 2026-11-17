---
cairn: log
change: a-search-page-resumes-after-its-last-hit
landed: 2026-11-17
---

# A search page resumes after its last hit

CLI family audit of 2026-11-15, PIM-5, open at dcf49e2: a search page token was a row offset, so a message arriving between two pages repeated a hit and a removal hid one, while `search` refreshes the index before every page. SEARCH §8.2 describes a page as "after a cursor, the last hit it answered", and `item list` is keyset-paged already.

## What landed

- src/client/search.rs: `PimdirSearchCursor` (`token`, `from_token`), `PimdirSearchOptions::after` in place of `offset`, `PimdirSearchOutput::next` and `PimdirSearchThreadsOutput::next`; every hit carries its place, the answer is ordered by it and a page starts after the cursor.
- src/cli/search.rs: `nextPage` and `--page-token` carry the cursor's token.
- Breaking for the crate's consumers (himalaya, cardamum, calendula page through `offset`, the audit's TRI-9) and for a token the binary printed before.
- Tests: tests/search.rs `a_page_resumes_after_the_last_hit_it_answered`, every search vector walked one hit a page, tests/search_cli.rs `a_search_page_token_resumes_after_its_last_hit` (m4 printed twice on dcf49e2).
- Left: PIM-10, every page still evaluating and ordering the whole hit set.

## Capabilities moved

search ("A page resumes after the last hit it answered", added).

In the working tree on io-pimdir dcf49e2 (0.15.1), uncommitted, no version bumped; CHANGELOG.md [Unreleased] names it.
