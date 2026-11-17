---
cairn: log
change: a-comment-closes-on-any-run-of-dashes
landed: 2026-11-17
---

# A comment closes on any run of dashes

CLI family audit of 2026-11-15, PIM-16, open at dcf49e2: the `text/html` reduction closed a comment on exactly two dashes before `>`, so `<!----------- SECTION ----------->`, common in newsletter templates, swallowed the rest of the body and a search for a word after it answered nothing while the coverage said complete.

## What landed

- src/search/html.rs: a comment closes on two dashes or more before `>`.
- src/search/extract.rs: `EXTRACTOR` is `io-pimdir 2` (SEARCH.md §6 leaves the HTML reduction to the indexer, so no draft moves): every index another revision built is extracted again once. Shipped with a-read-verb-takes-the-index-as-it-stands, so a read verb of one release never rebuilds the index another release keeps.
- Test: src/search/html.rs `a_comment_closes_on_any_run_of_dashes`.

## Capabilities moved

search ("An HTML comment closes on any run of dashes", added; "An index records the revisions of its extraction", modified: `io-pimdir 2`).

In the working tree on io-pimdir dcf49e2 (0.15.1), uncommitted, no version bumped; CHANGELOG.md [Unreleased] names it.
