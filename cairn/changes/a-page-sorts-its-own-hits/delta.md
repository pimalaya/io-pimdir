---
cairn: delta
change: a-page-sorts-its-own-hits
---

## ADDED Requirements

### Requirement: A page sorts its own hits
`PimdirIndex::search` SHALL select a page's hits among those following its cursor before sorting them, sorting the page alone, and answer exactly the hits a whole sorted answer holds after the cursor, in its order. A reader's overlay SHALL read the conflicts an item stands in only when it folds that item and an `update`, a `remove` or a `move` pending on it meets them (STORAGE §15.3, §15.4).

#### Scenario: Every order, cursor and limit
- src/client/search.rs `a_page_is_the_slice_of_the_sorted_answer_after_its_cursor`: 40 hits tying on every key but the public id, in the `date`, `relevance` and `name` orders, at every cursor and at limits 0, 1, 3 and 40; the overlay's suites (tests/overlay.rs, tests/item_conflict_settle.rs) unchanged

## MODIFIED Requirements

## REMOVED Requirements
