---
cairn: log
change: an-item-keeps-local-state
landed: 2026-11-21
---

# An item keeps local state

The storage half of pimdir daec31a: spec/ re-vendored at a272c65, which adds the two `collection:` cases of a-hit-is-placed-by-its-collections (spec/REVISION its full id), storage-draft-15 and search-draft-16 edited in place, so no version constant moves. The table `item_property(seq, key, value)`, its index, CHECKs and five triggers, the four reads, the two writes and the `evict_before` keeping an item holding a property come with the vendored migration and statements through build.rs, with no Rust edit.

## What landed

- src/property.rs, I/O-free: the bounds, `is_key`, `is_writable_key`, `is_value`, `fold` (Annex A.2's `fn` fold: `to_lowercase`, then a White_Space trim), `same`, and `change`, the fold of a `set-property` over the values held, which the owner's writes, the drain and the overlay share. A value removed and added again in the same spelling writes no row, so it stamps nothing, as STORAGE §4.5 has a value added again stamp nothing.
- `PimdirAction::SetProperty` and its v1 codec: the schema's members, one of `add` and `remove` stated, 1 to 64 distinct values each, and the text's byte bound on a value beside the schema's characters. tests/payloads.rs passes the 27 new cases, which the earlier build judged unknown.
- The drain (src/client/producer.rs, src/client/property.rs): a `set-property` applies right after its claim, before the capability backstop, as no source; parked when no live row of its collection holds the seq, or when the change leaves more than 64 values under the key; a receipt naming the seq.
- The reader's `list_item_properties`, `list_property_values`, `list_property_holders`, `list_value_holders`, and `item_properties(collection, seq)`, which folds the `set-property` rows pending on that collection when the reader overlays the queue. The owner's `add_property` and `remove_property` on its own account, refusing what a producer could not queue as `PimdirError::Property` (`property-invalid`).
- `pimdir property list|add|remove`, the CLI relaying `property-invalid`; `queue list` prints a `set-property` by its seq, key and counts, its values being user content.
- tests/search.rs seeds store.json's `properties` (the owner's `add_property`) and applies refresh.json's `property` write as a queued and drained `set-property`; `the_local_state_refresh_case_holds_in_the_store` checks that case's feed at every step and its 10 `label:` and `property:` answers from the store's reads. tests/generated/reader.rs maps the four reads the generator's oracle now states.

Capabilities moved: spec/store.md ("An item keeps local state", "The drain applies a set-property to the store alone", "A reader overlays a pending set-property on its collection", the eviction), spec/cli.md (the property verbs), spec/spec-fidelity.md (the revision, the pending list).

## Pending

tests/pending_vectors.txt names the 30 `label:` and `property:` cases of queries.json and the 7 steps of refresh.json's "an item keeps local state": the query compiler, which as-fast-as-notmuch's planner rewrites now, reads no such field yet. The store holds what they read, and the planner merge takes them off.

## Chosen here

- The owner's own `add_property` and `remove_property` hold the `set-property` terms (the label fold, the `pimdir/` reservation, the 64 values) rather than writing the statement as it stands, so no writer of this crate leaves two labels of one fold.
- The overlay is a call of its own, `item_properties(collection, seq)`: `list_item_properties` reads by the seq alone and names no collection, where §15.4 folds the rows queued on the collection a read shows. It checks the live row against the committed rows, not against the rows queued before it.
- A drain the CLI runs for `property add|remove` resolves its source as `--source`, else the store's one source; a store syncing none or several leaves the row queued, rather than draining as a source of no name.
