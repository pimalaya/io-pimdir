---
cairn: log
change: as-fast-as-notmuch
landed: 2026-11-22
---

# Posting lists, the coverage seeks, the floor

Round 4 of as-fast-as-notmuch, on the owner's three decisions of 2026-11-22. A whole-store query reads its temporal coverage through the canonical `covered_all`, three seeks on partial indexes of `sources` (pimdir storage-draft-16). index.db holds the crate's own `pimdir_fields`, posting lists of the titles' and From headers' words the refresh keeps beside `object_text` (spec/search.md, "A `subject:` or `from:` word reads the posting lists"); an index built before it is rebuilt by the indexer. Both schema checks compare bytes over `sqlite_schema` read unsorted; the term tables are declared on first use; snippets cut this build's own chunks (no `chunks_of` a hit) and read a blob by its stated length; a thread met by its only matching member reads no members; a page binds its candidates as JSON. A rebuilt index now drops its hit columns, whose stamp a reused inode could match.

On the 1M Enron store pimdir leads notmuch 0.40 on count, page and threads for 14 of 17 queries, every answer equal to b080e14's; rare word, folder and combined trail by 0.1 to 2 ms (the SQLite schema load, per-hit body reads, FTS5's joint `AND`). The owner closed the change as good enough for v1 that day. Numbers, what was tried and dropped: changes/as-fast-as-notmuch/round-4.md.
