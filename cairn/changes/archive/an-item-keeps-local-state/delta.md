---
cairn: delta
change: an-item-keeps-local-state
---

# Delta

## ADDED Requirements

### Requirement: An item keeps local state
An item's properties (STORAGE §4.3) SHALL be the rows of `item_property(seq, key, value)` the vendored migration creates, keyed by the public id, so a property holds in every collection and account filing the seq, through a retention, a restore and a rekey, and goes with the last row holding the seq, by the canonical triggers, which also refuse a key, a value or a 65th value under one key past its bound and stamp every row holding the seq on a change. `io_pimdir::property` SHALL hold the rules with no I/O: `is_key`, `is_writable_key` (`pimdir/label` or a key outside `pimdir/`), `is_value` (1 to 1,024 bytes of UTF-8 without NUL), `fold` (a label lowercased by the full Unicode mapping, then trimmed of White_Space, as Annex A.2 folds `fn`), `same` (folded under `LABEL`, byte for byte under any other key) and `change`, the one fold of a `set-property` over the values held: its removes first, each taking the value held that is the same, then its adds in order, each adding nothing when a value held or added before it is the same, answering the rows written (a value removed and added again in the same spelling in neither) or the overflow past 64 values. `PimdirReader::list_item_properties(seq)`, `list_property_values(key)`, `list_property_holders(key)` and `list_value_holders(key, value)` SHALL run the canonical reads (§14.1). `PimdirStore::add_property(seq, key, value)` and `remove_property(seq, key, value)` SHALL write on the owner's own account on a `set-property`'s terms, in a transaction of their own, answering whether a row moved, and refuse a key the store or a producer cannot write, a value past its bounds and a 65th value as `PimdirError::Property` (`property-invalid`), a seq no row holds being refused by the schema. The sync never reads a property (SYNC.md §3).

#### Scenario: A label compares folded
- tests/properties.rs `a_label_compares_folded_and_is_respelled_by_a_remove_and_an_add`.

#### Scenario: Kept by a retention and an eviction, gone with the last row
- tests/properties.rs `a_property_outlives_a_retention_and_an_eviction_and_goes_with_the_last_row`.

#### Scenario: The owner writes nothing past a bound
- tests/properties.rs `the_owner_writes_no_property_the_store_cannot_hold`.

### Requirement: The drain applies a set-property to the store alone
`PimdirAction::SetProperty { seq, key, add, remove }` SHALL read and write the v1 payload of payloads/set-property.v1.schema.json, `add` and `remove` empty for a member the payload leaves out, one of them stated; a value past 1,024 bytes of UTF-8 SHALL be malformed though the schema counts characters (STORAGE §15.3), and a key under `pimdir/` other than `pimdir/label` malformed by the schema, so a producer refuses it at the enqueue. The drain SHALL apply one as no source and before the capability backstop, needing no capability (§15.2, §15.6): parked when no live row of its row's collection holds the seq, or when `change` overflows; else its rows written by `remove_property` and `add_property` and a receipt naming the seq. It stages no mutation and derives no push (SYNC.md §2).

#### Scenario: Applied as no source, parked where the owner parks it
- tests/properties.rs `a_set_property_applies_as_no_source_and_names_its_item`, `a_seq_no_live_row_of_its_collection_holds_parks_and_so_does_a_65th_value`, `a_producer_refuses_a_key_pimdir_keeps`.

#### Scenario: The payload vectors
- tests/payloads.rs, the 27 `set-property` cases of vectors/payloads.json.

### Requirement: A reader overlays a pending set-property on its collection
`PimdirReader::item_properties(collection, seq)` SHALL answer `list_item_properties(seq)` with, when the reader overlays the queue, every `set-property` pending on `collection` for that seq folded in append order by `change` (STORAGE §15.4): nothing folded when no live row of `collection` holds the seq, and a row overflowing folded into nothing. A row pending on another collection holding the seq shows once applied. An item listing's fold is unchanged: a `set-property` changes no item row a listing shows.

#### Scenario: A pending relabel
- tests/properties.rs `the_overlay_folds_a_pending_set_property_on_its_own_collection_alone`.

### Requirement: The property verbs
`pimdir property list SEQ` SHALL print an item's properties as the store holds them (`pimdir-property-list`), and `pimdir property add|remove SEQ KEY VALUE...` SHALL stage one `set-property` through the producer role on `--collection`, else on the first collection holding the item live, then drain when the owner role is free and a source to drain as resolves (`--source`, or the store's one source), reading the outcome from the row's status: applied, queued, or refused with exit status 3 (`pimdir-property-add`, `pimdir-property-remove`). A value is user content, printed through the text sanitiser; `queue list` prints a `set-property` by its seq, key and value counts alone.

#### Scenario: A label added, listed and removed
- tests/cli_property.rs `a_label_is_added_listed_and_removed_through_the_queue`, `a_key_pimdir_keeps_is_refused_and_nothing_is_queued`.

## MODIFIED Requirements

### Requirement: The owner evicts below a date
An item whose seq holds a property SHALL stay, as one a person's reference names: no relisting restores local state (STORAGE §11.3, the vendored `evict_before`).

### Requirement: The vendored copy is the specification's, byte for byte
spec/REVISION names pimdir daec31a, storage-draft-15, sync-draft-15 and search-draft-16 edited in place. tests/pending_vectors.txt names the 30 `label:` and `property:` cases of queries.json and the 7 steps of refresh.json's "an item keeps local state", the fields the query compiler does not read yet; tests/search.rs seeds store.json's `properties` and applies refresh.json's `property` write as a `set-property` queued and drained, and checks that case's feed and its `label:` and `property:` answers from the store's own reads.

## REMOVED Requirements

None.
