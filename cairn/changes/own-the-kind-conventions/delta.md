---
cairn: delta
change: own-the-kind-conventions
---

## ADDED Requirements

### Requirement: The crate owns the per-kind derivations
The crate SHALL implement the per-kind conventions pimdir Annex A fixes: given a raw body, the `link_id` (including its fallback when the body carries no stable identity), the `meta` summary and the `sort_key`, one derivation per media type the format names.

A consumer SHALL derive through it rather than carry its own copy. Annex A is informative because the store never parses either value, but it is still an agreement every writer of one collection keeps: two writers disagreeing produce a collection whose rows one of them cannot render or order, and neither is in a position to notice, exactly as two hashes produce blobs neither finds.

The derivations SHALL live in the I/O-free core: they read no I/O, and a consumer running its own SQLite driver needs them without the std client. The store's own paths SHALL keep treating `meta` and `sort_key` as opaque values they ferry and never parse, which this module does not change: it is called by the writer, before the bytes reach the store.
