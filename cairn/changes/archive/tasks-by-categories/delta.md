---
cairn: delta
change: tasks-by-categories
---

# Delta

## ADDED Requirements

### store: items kept on the device

`PimdirStore::put_item(collection, body)` SHALL write a body into a collection no source syncs (STORAGE §14.3), of kind `text/vcard` or `text/calendar`: its key, summary and sort key derived as a synced write derives them (Annex A), the item at `Full`, its summary, its automatic references (§14.2) and its pins in one transaction, a held key taking the new body with its public id and flags kept, answering the link id and the `seq`. It SHALL refuse a collection a source syncs (`collection_sources` answering any) as `PimdirError::Synced` and any other kind, undeclared included, as `PimdirError::NotUnbound`, writing nothing. `client::TIME_COLLECTION` SHALL be the local time collection's id, `U+0001` and `time`; `PimdirStore::ensure_time_collection` SHALL declare it, kind `text/calendar` under no account and named `time`, idempotently, and answer it. `PimdirStore::delete_unbound` SHALL delete such an item with its summary and release its pins.

## MODIFIED Requirements

## REMOVED Requirements
