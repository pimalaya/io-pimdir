---
cairn: change
id: collection-display-name
status: landed
created: 2026-09-06
---

# A collection can be named, not only addressed

## Why

`collections.name` is declared by the canonical schema as the logical name (`INBOX`, `Contacts`) and this crate never writes it. The two statements that touch the column, `ensure_collection` and `set_collection_kind`, seed it from `:collection`, and there is no setter, so in every store this crate has written the name is a verbatim copy of the id.

An owner namespacing its ids therefore puts the prefix in the label as well. neverest's stores read `imap/Archives`, `carddav/Default`, `caldav/ED99C7C8-2741-11F1-9B88-2C202A48A29D`; the last is unreadable, DAV addressing a collection by a path segment servers routinely make a UUID. A consumer already paid for the gap: pimalaya-linux carries a private `app_collection_name` table and a resolver beside it, for want of a column the format declares.

The standard closed its half in pimdir's `collection-display-name`: §14 now states that a name is a label, never an address, and `set_collection_name` writes it.

## What

- Re-vendor spec/queries/storage/owner/ with the new statement, so `sql::SET_COLLECTION_NAME` is generated and tests/spec_drift.rs stays byte-exact against the sibling checkout.
- `PimdirSourceStore::set_collection_name(collection, name)`, beside `ensure_collection` and `set_collection_account` and binding the handle's own account the way they do.
- tests/collection_name.rs: the label moves while the id, the kind and the account do not, and moving it stamps the collection in the change feed while restating it does not.

## Not in scope

`color`, `description` and `sort_order` keep the same missing-setter gap; no consumer reads them yet. Nothing migrates: the column has been in 0001 from the start.
