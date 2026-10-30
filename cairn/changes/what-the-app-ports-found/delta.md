---
cairn: delta
change: what-the-app-ports-found
---

## ADDED Requirements

### Requirement: An action on an item no source binds is the store's to settle
A `remove` retains it; in a collection no source syncs an `update` and a `set-flags` apply to the store; anything else parks; nothing is skipped.

### Requirement: The drain reports the bodies a policy kept aside
`PimdirDrainReport::overruled`, one `PimdirDrainOverruled { id, collection, link_id }` per body kept aside.

## MODIFIED Requirements

### Requirement: Blobs are content-addressed and sharded
`reader` answers a `PimdirBlobReader` failing the read at its end when the bytes do not hash to the name.

### Requirement: A store this crate may not write is never written
The reader, the producer and `open_current` refuse a foreign database as `Foreign`.

### Requirement: A reader may overlay a collection's pending actions
An `update` whose `if_match` names another body than the one held at that point of the fold is not folded.

### Requirement: A write staged with its body stages nothing when refused
`enqueue_with_body_checked` answers the partial supports beside the row id.
