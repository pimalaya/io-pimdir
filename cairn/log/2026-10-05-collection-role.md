---
cairn: log
change: collection-role
date: 2026-10-05
---

# A collection keeps the role its source states

pimdir's `collection-role` change (draft-04, not tagged yet) gives `collections` a `role`: what the source states a collection is for, so a reader learns the sent folder or the default calendar from the store. MOA asked its users to confirm each folder for want of it.

## What landed

- **Vendored spec** (capability `spec-fidelity`): 0001_init.sql with the column, `collections_by_role`, `collections_role_moves` and the stamp trigger watching `role`; set_collection_role.sql; `role` last in list_collections and list_collections_by_account. tests/spec_drift.rs holds the copy.
- **`PimdirStore::set_collection_role`** and **`PimdirCollection::role`** (capability `store`); `pimdir collection list` shows it.
- **Reconcile on open**: the column cut out of the canonical DDL and added by `ALTER TABLE`, the index and the moving trigger created, the stamp trigger recreated when its body predates the column. A reader of a store not reconciled yet reads `NULL` (`collections_sql`).
- **tests/collection_role.rs**: the move and its two stamps, the vocabulary per kind, one holder per account, the reconcile of a store written without the column.

## Verification

`cargo test --all-features` green but for `hub_property::hub_interleavings_converge_across_sources`, a proptest case that fails on the parent commit too (a staged body on a new link never becoming the shared one after `ServerAdd`, `Add`, `Sync`, `Edit`, `Sync`, `Edit`); not touched here, not recorded as a regression seed.

Capabilities moved: `store`, `spec-fidelity`.
