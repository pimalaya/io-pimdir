---
cairn: log
change: as-fast-as-notmuch
landed: 2026-11-21
---

# The hit columns

Counts, dated pages and threads read index.hits, a cache of flat columns beside index.db written by the refresh, before any plan is drawn: text terms read FTS5 rowids with no placement join, flag and date terms one pass over the records or a test per hit, an `and` of text terms one FTS5 expression while `split` is empty, a page's candidates sorted exactly by `met_rows`. On the 1M tier (corpus x2, 1,034,802 messages) counts went from 24 to 210 ms to 4 to 55, pages and threads of selective terms from 35 to 124 ms to 10 to 16; every count and page equal to b080e14's. pimdir now beats notmuch 0.40 on 8 of the 17 measured queries on all three of count, page and threads, and on pages and threads for 12; it trails on the process floor (rare word, folder), `@` address terms, combined and from name, and on a few counts by 1 to 3 ms. Numbers and what is left in changes/as-fast-as-notmuch/round-2.md.

The cache is io-pimdir's own, not a canonical file: another implementation ignores it, and its stamp (with index.db's inode) refuses it once anyone else writes or rebuilds the index. rustix gains its `mm` feature for the map.
