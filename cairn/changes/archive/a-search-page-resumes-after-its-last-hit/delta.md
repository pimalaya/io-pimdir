---
cairn: delta
change: a-search-page-resumes-after-its-last-hit
---

## ADDED Requirements

### Requirement: A page resumes after the last hit it answered
`PimdirSearchOptions::after` SHALL name where a page starts: a `PimdirSearchCursor`, the place in the query's order of the last hit an earlier page answered (its sort key and whether undated, its rank, or its folded title, then its `seq` and account), which `PimdirSearchOutput::next` and `PimdirSearchThreadsOutput::next` answer while more follows; a threaded listing's cursor is the first hit of its last thread. A page SHALL answer the hits after that place, so a hit arriving or leaving between two pages neither repeats nor hides another, and every page walked to the end answers the hits and the order one whole answer does (SEARCH §8.2). `token` writes a cursor as printable ASCII and `from_token` reads it back; the binary's `nextPage` and `--page-token` carry it, an unreadable one being `page-token-invalid`. The offset of 0.15 is gone.

#### Scenario: An arrival and two removals between pages
- GIVEN five messages, the first page of two answering m5 and m4
- WHEN m6 arrives and the next page is read after the cursor, then m6 and m5 leave
- THEN the pages answer m3 and m2, then m1 and no cursor (tests/search.rs `a_page_resumes_after_the_last_hit_it_answered`, tests/search_cli.rs `a_search_page_token_resumes_after_its_last_hit`; every search vector is also walked one hit a page)

## MODIFIED Requirements

## REMOVED Requirements
