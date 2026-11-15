---
cairn: delta
change: a-pending-create-is-withdrawn-whatever-the-rights
---

## ADDED Requirements

### Requirement: A pending create is withdrawn whatever its source may delete
The gate SHALL leave out of an item's sources, for a `remove`, a `move` or a `copy`, a source binding the item as a pending create alone (`held` 0): a remove asks it nothing, a move or a copy asks it, when it syncs the target, what an `add` carrying the item's flags needs there. A `set-flags` and an `update` read every source binding the item; a delivered create is held.

#### Scenario: A message its server refused, deleted while deletes are off
- GIVEN a pending create of graph in INBOX, graph syncing Archive and declaring `mail.message.remove` and `mail.message.move` `none`
- WHEN a producer checks a remove of it and enqueues a move of it to Archive, and the owner drains
- THEN both pass and the drain applies the move, parking nothing, while a remove and a move of a message graph holds are refused (tests/capabilities.rs `a_pending_create_is_withdrawn_whatever_its_source_may_delete`)
