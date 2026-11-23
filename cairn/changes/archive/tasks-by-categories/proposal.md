---
cairn: change
id: tasks-by-categories
status: landed
created: 2026-10-28
---

# Tasks by categories, and items in a local collection

## Why

pimdir's change tasks-by-categories states the GTD convention as queries over columns this crate already derives, and a local time collection no source syncs (`U+0001time`). The queries need nothing new; the collection does. The owner can put a file into a collection no source syncs (`PimdirStore::put_file`) and delete an unbound item (`delete_unbound`), but has no way to write a calendar item there, so unfog's local time strategy has no writer.

## What

- **Writes to an unbound collection**: `PimdirStore::put_item(collection, body)` for contacts and calendar items (the key derived from the body, never passed beside it, so the two cannot disagree; mail is a source's and a file's summary the caller's, `put_file`), deriving the summary and the references as a synced write does, in one transaction (STORAGE §5), refused on a collection a source syncs. An update is a new body under the same link id. `delete_unbound` already removes it.
- **The local time collection**: `PimdirStore::ensure_time_collection()` declares `U+0001time` (`text/calendar`, no account) and returns its id; the constant is public so readers find it without declaring it.
- **Vectors**: spec/ re-vendored; tests/search.rs answers the four view queries; tests/unbound_items.rs writes, restates and deletes a VEVENT in the time collection and shows no source syncs it.

## Out

- GTD logic: views, capture and clarify are the consumers'.
- Durations and reports: unfog's.
