---
cairn: delta
change: reader-role
---

## ADDED Requirements

### Requirement: The store has a reader role of its own
A consumer that only reads SHALL be able to hold a handle that can only read. `PimdirReader` SHALL open a store read-only, take no lock (SPEC §8), and carry the read surface alone: it SHALL NOT expose the drain, the object sweep, a purge, a retention write, or an enqueue. A reader and an owner SHALL run the same statements, so the two roles never disagree about what the store holds.

#### Scenario: A reader cannot drain
- GIVEN a consumer holding a `PimdirReader`
- WHEN it looks for a way to apply a queued action
- THEN there is none on the type, and the store's owner remains the only process that applies the queue

#### Scenario: A reader runs beside a sync
- GIVEN a store an owner holds
- WHEN a reader opens it
- THEN it opens immediately, taking no lock and waiting on nothing

### Requirement: A reader may overlay a collection's pending actions
A reader built with the overlay SHALL project a collection's pending actions over its committed items, in row `id` order, so a consumer sees its own staged writes before the owner applies them (SPEC §15.4). The fold SHALL cover the kinds that address an existing item: `set-flags`, `remove`, `move`, `copy` and `update`. Each keeps the item's public id, a `seq` following the link id store-wide (SPEC §9.1), so the overlay SHALL NOT invent an identifier.

A pending `add` SHALL NOT be projected as an item: it has no `seq` until the owner applies it, and it is a request to create a message rather than one. The reader SHALL report pending creates apart, as rows and as a count.

A parked row SHALL NOT overlay, its error asserting that it will not be applied without an operator.

The overlay SHALL be chosen when the reader is built rather than per call, so one handle cannot answer two ways about one collection.

#### Scenario: A staged flag is visible before the sync
- GIVEN a producer has enqueued `set-flags` on a listed item
- WHEN an overlaying reader lists the collection
- THEN the item carries the staged flags, and a reader built without the overlay shows the committed ones

#### Scenario: Two actions on one item fold in order
- GIVEN a queued `set-flags` followed by a queued `move` on the same `seq`
- WHEN an overlaying reader lists both collections
- THEN the item shows the staged flags in the target collection, the later row deciding where it sits

#### Scenario: A queued create is counted, not listed
- GIVEN a queued `add` carrying a link id the store has never seen
- WHEN an overlaying reader lists the collection
- THEN no item is added to the listing, and the reader reports one pending create

### Requirement: Cancelling reaches a non-owner without an owner handle
Cancelling a queued row is an owner write (SPEC §15.5) and is the only retraction a create has. A consumer SHALL be able to perform it through a scoped operation that opens the store as owner, cancels one row, and releases the lock before returning, without ever holding a type that can drain or sweep. A store another process owns SHALL fail fast with `PimdirError::Owned`, never wait.

#### Scenario: Cancel while a sync runs
- GIVEN a store whose owner is running a sync
- WHEN a consumer cancels a queued row through the scoped operation
- THEN it fails immediately as owned, the action still pending and possibly already applied

## MODIFIED Requirements

### Requirement: Reads open the store read-only
`PimdirStore::open_read_only` is deprecated in favour of `PimdirReader::open`. It returned an owner-shaped handle whose writes failed at the SQLite layer, which made the role a run-time property of a call rather than a compile-time property of the handle.
