---
cairn: delta
change: only-the-sync-engine-migrates
---

## MODIFIED Requirements

### Requirement: A queued action can be cancelled or acknowledged
`PimdirStore::cancel_action(dir, id)` SHALL open the store through `open_current`, never `open`: a consumer cancelling a row owns the store for one write and SHALL NOT create, migrate or reconcile it. A store of another `user_version` or shape is refused as `PimdirError::NotCurrent`, left as it was; a store it cannot find stays `PimdirError::Uncreated`.

#### Scenario: A cancel on a store another revision keeps
- GIVEN a store whose `task_summary` carries a column a later revision added, and a queued row
- WHEN a consumer cancels a row through `cancel_action`
- THEN it answers `NotCurrent(Shape { object: "task_summary" })` and the table and the row are left as they were (tests/open_current.rs `a_cancel_leaves_a_store_needing_a_reconcile_as_it_was`)

### Requirement: Terminal operations take the owner role directly
The CLI's owner role SHALL open the store through `open_current`: the CLI is never the store's sync engine, so it SHALL NOT create, migrate or reconcile a store, and a store of another version or shape fails the verb with `NotCurrent`'s sentence, written nothing. A verb that enqueues then drains (`item restore`, `tag`) SHALL leave its action queued and report it `queued`, as when another process owns the store.

#### Scenario: An older binary over a newer store
- GIVEN a store whose schema text differs from the binary's
- WHEN `pimdir queue cancel` or `pimdir gc` runs
- THEN it fails and the store is left as it was (tests/cli_owner.rs `an_owner_verb_leaves_a_store_of_another_revision_as_it_was`); `pimdir item restore` reports its action `queued` and exits 0 (`a_restore_over_a_store_of_another_revision_stays_queued`)
