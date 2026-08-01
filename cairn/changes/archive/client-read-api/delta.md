---
cairn: change
change: client-read-api
---

# Delta

## ADDED Requirements

### Requirement: A client reads the store by indexed, paginated getters
The store SHALL expose a read-only query surface for a client projecting the
store as a local backend, distinct from the sync seam's load-all:

- `list_collections` SHALL return every collection's `id`, `kind`, `name`,
  `parent`, `color`, `description` and `sort_order`.
- `list_items` SHALL return a page of a collection's **live** items (`deleted =
  0`), keyset-paginated by `link_id` (`link_id > after`, ordered by `link_id`,
  at most `limit`), each carrying its `link_id`, flags, raw `meta`, object hash
  and detail `level`.
- `get_item` SHALL return one live item by `(collection, link_id)`, or nothing.
- `count_items` SHALL return a collection's live item count.

These reads are kind-agnostic (raw `meta`, string flags, opaque object hash) and
observe only — they never mutate; all writes remain io-replica `ReplicaWriteOp`s
through `write`.

### Requirement: Reads are availability-aware
A read result SHALL carry each item's detail `level` (`Probed`/`Meta`/`Full`), so
a caller knows a body is not local (`level < Full`, `object` absent) without
probing the blob store, and can trigger a hydrate through the sync engine rather
than treating the absence as data loss.

## MODIFIED Requirements

## REMOVED Requirements
