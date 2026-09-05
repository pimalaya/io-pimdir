---
cairn: delta
change: store-spec-conformance
---

## ADDED Requirements

### Requirement: The two schema stamps must agree
`PRAGMA user_version` and `store_meta.version` mirror one another (spec §4.2), so a store where they differ is corrupt rather than a store at either version. Both the owner open and the read-only open SHALL compare them and refuse a disagreement with `PimdirError::VersionMismatch`.

A store carrying no `store_meta` row SHALL be left alone: the row is seeded by whoever created the schema, and refusing there would make a missing stamp unrepairable.

### Requirement: An unknown flag set is stored as NULL
The `flags` column keeps two absences apart (spec §13): `NULL` means nothing has read the item's markers, `'[]'` means it is known to carry none. The store SHALL write `NULL` for an unknown set and decode `NULL` back to one, so a probed placement never claims to carry no markers.

In a queue payload an unknown set SHALL encode as `null` rather than `[]`, since an action states an intent: every payload the format defines carries a known set, and an unknown one must not read as a deliberate clearing of every flag.


### Requirement: A store predating the rename cascades is refused
Every foreign key onto a renamable parent carries `ON UPDATE CASCADE` (spec §14), which no `ALTER TABLE` can add to a store that lacks it. Reconciliation therefore cannot reach it, and spec §6's other branch applies: both opens SHALL check the cascade on every such key and refuse a store without it with `PimdirError::Unreconcilable`, naming the table.

Refusing is what makes the limitation legible. Opened anyway, such a store works until something renames a collection, and then SQLite refuses the rename one dependent row down, so a server-side rename or an account rename can never be applied. Recreating the store costs a resync of what the format calls a derived cache.

## MODIFIED Requirements

### Requirement: A store from an earlier draft of the current version is reconciled on open
While the pimdir spec is `draft`, a schema change MAY be folded into version 1
rather than added as a new version (spec §6). A store written by an earlier
draft is then stamped with the current `user_version` yet lacks the folded-in
columns, so the version check alone cannot detect it.

On open, the store SHALL reconcile its shape: every folded-in column found
missing SHALL be added (`ALTER TABLE … ADD COLUMN`, which requires the column to
be nullable or carry a constant default), guarded so the check is a no-op for an
up-to-date store, together with any index over a folded-in column. The set of
folded-in columns SHALL be kept complete as further columns are folded in;
`items.sort_key` and its `items_by_sort` index are part of it. Failing a later
query on a missing column is not acceptable. This requirement lapses when the
spec leaves `draft` and versions are frozen.
