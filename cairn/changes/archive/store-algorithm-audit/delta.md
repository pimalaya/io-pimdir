---
cairn: delta
change: store-algorithm-audit
---

# Delta

## ADDED Requirements

### Requirement: A write reads only the rows its batch names
`write` SHALL load the hub narrowed to the link ids its batch touches (`LOAD_ITEMS_BY_LINK`, `LOAD_BINDINGS_BY_LINK`), resolving each dropped handle to its link id first (`LINK_FOR_HANDLE`, served by `bindings_by_handle`).

The diff only ever names rows the batch named, so the rest of the collection would be read, cloned and merged to conclude that nothing changed, and that read, not the writes, is what a small write actually costs: it grows with the mailbox instead of with the batch. Both sides of the diff are narrowed the same way, so every comparison the persistence step makes and every object reference the refcount step counts sees exactly what it would have seen in full.

#### Scenario: One flag on one message in a large mailbox
- GIVEN a collection holding many items
- WHEN a batch upserts one placement
- THEN the rows read do not grow with the collection

### Requirement: Every column the update writes is in the diff
The row diff that decides whether an item needs an `UPDATE` SHALL compare every column `UPDATE_ITEM` writes. A column left out is a column that can never change again: the diff reports the row unchanged and no statement is issued for it.

#### Scenario: A restated sort key
- GIVEN a stored item whose key was derived once
- WHEN a write carries a different key and nothing else changed
- THEN the stored key becomes the new one

### Requirement: A descending page reserves no key
The first descending page SHALL bind a `NULL` cursor rather than a key no real one is expected to outrank. A sort key is arbitrary text a writer derives, so no value is reserved: a sentinel hides everything sorting above it from every descending page, permanently, while the count still reports it.

#### Scenario: A key above the sentinel
- GIVEN an item whose key outranks any fixed sentinel
- WHEN the collection is paged in both directions
- THEN both directions page every item

### Requirement: The drain claims a row before applying it
`drain_collection` SHALL delete the queue row it is about to apply as the **first** statement of the applying transaction (`CLAIM_ACTION`), and skip the action when that delete returns nothing.

The pending rows are read outside any transaction, so a second owner may hold the same list; deleting at the end has both apply the row, and `add` and `copy` are not idempotent. Claiming first makes exactly-once a property of the statement rather than a convention about who runs the drain.

### Requirement: A blob rename is durable
Writing a blob SHALL sync the shard directory after the rename. Syncing the file makes its bytes durable and says nothing about the name that reaches them, while the database commit is durable, so without it a crash can leave a committed row pointing at a body that never arrived: the one asymmetry the write order exists to prevent.

### Requirement: An unreadable flag set holds no opinion
A `flags` column this crate cannot decode SHALL read as unknown, never as a known-empty set. Malformed JSON is a column written by something whose format this does not share, or a corrupted one, and neither is evidence about the item's markers. Reading it as known-empty makes it an authoritative "this item carries no markers", which the merge takes as one side's opinion: it clears every marker the other side reports and persists the result, so a read failure becomes permanent loss.

## MODIFIED Requirements

### Requirement: Objects are swept by a predicate an index serves
The sweep SHALL test `refcount <= 0`, matching the partial index `objects_garbage` exactly: a count a double release drove negative is then collected rather than leaking for ever with nothing reporting it, and neither half of the sweep scans the whole table on every write transaction.

### Requirement: The store timestamp is the database's own
`store_meta.created_at` SHALL be written by SQLite in the RFC 3339 form the column is declared to hold, as the retirement clock already does. Reading a clock and formatting it by hand is what had the column holding epoch milliseconds, and the empty string when the clock predated the epoch.

## REMOVED Requirements
