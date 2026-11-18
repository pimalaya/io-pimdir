---
cairn: log
change: a-page-sorts-its-own-hits
landed: 2026-11-18
---

# A page sorts its own hits

CLI family analysis of 2026-11-15, PIM-10, re-checked at 117b5e7: a search page sorted the whole hit set, and an overlaid read read the conflicts of every item a pending `update`, `remove` or `move` names in its collection before folding a page.

## What landed

- src/client/search.rs: `PimdirIndex::search` keeps the hits after the cursor, selects the page (`select_nth_unstable_by`) and sorts it alone; `found` stays for `threads` and the tag plan, which walk the whole answer.
- src/client/reader.rs: `folded` reads an item's conflicts when the overlay folds it and a pending `update`, `remove` or `move` meets them; `PimdirPending` holds no conflicts any more.
- Test: src/client/search.rs `a_page_is_the_slice_of_the_sorted_answer_after_its_cursor`. No behaviour change: the overlay and search suites pass as before. Not re-timed.
- Left, a format question each: the statement per hit of a threaded listing, and the evaluation of the whole query per page.

## Capabilities moved

search ("A page sorts its own hits", added).

In the working tree on io-pimdir 117b5e7, uncommitted, no version bumped; CHANGELOG.md [Unreleased] names it.
