---
cairn: delta
change: a-page-lands-in-few-statements
---

## ADDED Requirements

### Requirement: An item takes one stamp per write batch
A write batch SHALL leave each item it writes with one stamp, however many of its rows (the item, its flags, its summary) the batch writes. The stamps SHALL keep STORAGE §4.5's guarantees: unique, increasing across commits, a page ordered by `changed` total. The rows a batch writes SHALL be those the per-row stamping writes, stamps aside. No item SHALL hold a pending stamp once its batch commits.

#### Scenario: A fill page
- GIVEN a mailbox page of 500 messages the store never held, each with two flags
- WHEN the page's write batch is applied
- THEN each new item holds one stamp, the 500 stamps distinct and above every stamp before the batch
- AND no item holds a pending stamp
- AND the rows equal those the per-row stamping writes for the same batch, stamps aside

#### Scenario: A writer that does not opt in
- GIVEN a write that inserts an item without a pending stamp
- WHEN it adds a flag to that item
- THEN the item is restamped as today

## MODIFIED Requirements

## REMOVED Requirements
