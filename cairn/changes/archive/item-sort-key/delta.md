---
cairn: delta
change: item-sort-key
---

## ADDED Requirements

### Requirement: An item carries an ordering key
`items` SHALL carry `sort_key` (TEXT, `NOT NULL DEFAULT ''`): the item's
position in its collection's natural order, written by the same writer that
writes `meta`. The store SHALL NOT derive it, and in particular SHALL NOT parse
`meta` to obtain it, since that would make the summary blob normative JSON with
a reserved key for every kind.

`''` means unknown and is the default, so an item is orderable from the moment
it exists. It sorts before every real key ascending and after every real key
descending.

### Requirement: A collection pages in its own order
`PimdirStore` SHALL expose `list_items_page_asc` and `list_items_page_desc`,
keyset pages ordered by `(sort_key, seq)`. The cursor SHALL be the pair, since a
sort key is not unique and `seq` is what makes the page total, and the first
page SHALL be requestable with no cursor rather than with a caller-invented
sentinel.

The index `items_by_sort ON items(collection, sort_key, seq)` SHALL serve both
as an index seek, not a scan.

#### Scenario: A page is total across a tie
- GIVEN two items sharing a sort key
- WHEN a collection is paged in either direction with a limit that splits them
- THEN each item appears exactly once across the pages, and none is skipped

### Requirement: A key can be restated without refetching
`PimdirStore` SHALL expose `set_sort_key(collection, link_id, sort_key)`, so a
consumer can derive keys for items already stored, whether because its kind had
no convention when they were written or because the sync engine does not carry
the key inline yet.

## MODIFIED Requirements

### Requirement: A client reads the store by indexed, paginated getters
The item read surface SHALL return `sort_key` alongside the existing columns,
and `list_items_page` SHALL be documented as the link-id-ordered sweep page: the
right page for a pass that must see every item exactly once, and not the one a
reader presenting a list should use.

## REMOVED Requirements

None.
