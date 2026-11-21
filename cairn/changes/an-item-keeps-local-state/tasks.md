---
cairn: tasks
change: an-item-keeps-local-state
---

# Tasks

- [x] Re-vendor spec/ at pimdir daec31a, spec/REVISION its full id
- [x] src/property.rs: bounds, key and value rules, the label fold, `change`
- [x] `PimdirAction::SetProperty` and its codec; tests/payloads.rs passes the 27 new cases
- [x] The drain's apply, its park reasons and its receipt (src/client/property.rs, src/client/producer.rs)
- [x] The four reads, the overlay `item_properties`, the owner's `add_property` and `remove_property`, `PimdirError::Property`
- [x] tests/properties.rs; tests/search.rs seeds `properties`, applies the `property` write and checks the store's half of the refresh case
- [x] The `label:` and `property:` vector cases listed in tests/pending_vectors.txt (30 queries, the 7 steps of the refresh case)
- [x] `pimdir property list|add|remove`, tests/cli_property.rs
- [x] cairn/spec folded (store.md, cli.md, spec-fidelity.md), the log, CHANGELOG.md
- [x] `label:` and `property:` in the query compiler, the pending cases off the list (as-fast-as-notmuch, phase 2)
