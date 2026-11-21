---
cairn: log
change: as-fast-as-notmuch
landed: 2026-11-21
---

# Text counts off the index

The text-count gap of cairn/changes/as-fast-as-notmuch (tasks.md, phase 2). On the 517k bench `enron` was counted in 506 ms against notmuch's 48: 330 reading its 498,533 matches into `temp.term_match`, 28 making them distinct, 285 seeking each hit's live placement in the store (`placed`).

- **One read transaction** (src/client/search.rs `count`): the count and its coverage answer one state of the store and the index, where each statement read its own before.
- **No store seek once the index is current** (`INDEX_CURRENT`, src/sql/search.rs: no stamp above the index's cursor and the store's purge count the one it reconciled). The index then holds one placement per live item (SEARCH §3; a delete stamps the item, and a purge, a collection removal or a rename counts a purge), so a hit read from a text set is live and in an unbounded scope: `Compiled::count_set` drops `placed` for it, as it did for a system flag's set. A lagging index keeps the seek, which SEARCH §8.2 needs ("a hit the store no longer holds is never answered").
- **Text terms alone counted as bitmaps** (`Compiled::bitwise`, `PimdirIndex::count_bitwise`): over every collection, terms combined by `and` holding a positive operand (negated ones taken away) and `or` of positive ones, at any depth, each term's placements read once through the canonical `match_objects` and `match_summaries`, unranked, into a bitmap of public ids per account, a collection's account read only when the store holds several (`COUNT_ACCOUNTS`, `COLLECTION_ACCOUNTS`). No estimate, plan or table of matches. A forced driver, a scope term, another kind of term, a negation needing every hit or a store past `BITMAP_SEQ_MAX` takes the plan.

The vectors count each case unforced, which takes the bitmaps on a current index, and under each forced driver, which takes the plan; the lagging steps of refresh.json take the seek.

## Proof

- Every mapping of a collection to account 0 fails `every_search_vector_answers` and `every_refresh_vector_answers` (the message filed in two accounts); a count one too high fails both.
- A negated operand combined as a positive one (`&= other` for `&= !other`) passed every vector: none held a negated text term beside text terms alone. pimdir c5f1e85 adds `(stand-up or reunion) not nothing`, `[home, 2]`, which fails under it; spec/ is re-vendored there and spec/REVISION names it.
- An `or` combined as an `and` fails `every_search_vector_answers`.

## Measured

cairn/changes/as-fast-as-notmuch/results-2026-11-21.md, "Counts, text off the index": `enron` 506 to 107 ms (notmuch 48), `california` 60 to 21 (6.2), the phrase 42 to 19 (10), `power california` 125 to 44 (5.5), `california not gas` 129 to 47 (6.4), `deriv*` 24 to 12 (3.9), `from:dasovich` 20 to 9 (3), `subject:meeting` 37 to 17 (3.8), `is:unread california` 80 to 61 (4.9); the rest unchanged.

## Left

The seek from a body to its placement, about 0.2 µs a matched row, is the floor of this design. FTS5's own `and` answers the two words in 6 ms but matches a document, not a hit; it would be exact for every hit whose placements name one document if the index recorded the hits naming several, a canonical schema change for the owner.

Capabilities moved: search.md (a new requirement, "A count of text terms reads the index alone once it is current"), spec-fidelity.md (the revision).
