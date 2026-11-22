---
cairn: log
change: as-fast-as-notmuch
landed: 2026-11-22
---

# Addresses off the hit columns, the open floor

`@` address terms are answered off index.hits (format PIMHITS3, which maps each store item id to its placement): `store.item_address` read by its address index, no join to items or collections. The store is read through SQLite's shared cache, so the index's ATTACH reuses the reader's parsed schema (0.75 to 0.03 ms), and both schema checks compare against constants build.rs derives from the migrations, word by word with no allocation (the migration parser moved to src/client/schema/declared.rs, compiled by build.rs and by the crate's tests). Snippets fold ASCII text in one pass, and a page off the columns reads a plain term matching at most 1,024 bodies into bitmaps rather than probing each hit. On the 1M tier every count and page equals b080e14's; from:jeff.dasovich@enron.com went 23.6 / 27.8 / 29.3 ms to 5.9 / 7.5 / 8.9 (notmuch 13.3 / 17.3 / 19.1), and pimdir now leads notmuch 0.40 on all three measures for 10 of the 17 queries. Numbers and what is left in changes/as-fast-as-notmuch/round-3.md.

Second step the same day (04d3211): snippets read the body through `PimdirExtract::text` (no address fields, no attachments walk, which parsed the message twice) and the store schema check walks `sqlite_schema` in name order borrowing each text. Measured head to head in one hyperfine run per measure (the machine's governor drifts 1.5x within an hour), pimdir leads notmuch on all three measures for 11 of 17 queries; from name, combined, rare word and folder trail by 0.4 to 2 ms, the rest of the gap in round-3.md with the three owner decisions that would close it.
