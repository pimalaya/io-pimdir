---
cairn: delta
change: collection-display-name
---

# Delta

## ADDED Requirements

### Requirement: A collection's name is a label, never an address
`collections.name` SHALL be writable independently of `collections.id`: `set_collection_name(collection, name)` moves the label and touches neither the id every foreign key cascades on, the declared kind, nor the account. An owner namespacing its ids SHALL record the bare name there, since the separator is its own convention and a reader cannot strip one it does not know. Nothing keys on the column, so moving it costs a label and never a re-sync; it is observable, so the collection takes a new `changed` stamp and a reader on the feed re-renders.

## MODIFIED Requirements

None.

## REMOVED Requirements

None.
