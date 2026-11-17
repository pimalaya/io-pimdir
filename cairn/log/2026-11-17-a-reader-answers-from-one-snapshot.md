---
cairn: log
change: a-reader-answers-from-one-snapshot
landed: 2026-11-17
---

# A reader answers from one snapshot

CLI family audit of 2026-11-15, PIM-11, open at dcf49e2, four points: a reader answer spanning several statements opened no read transaction; `evict_before` and `release_before` take an unvalidated cutoff; a store left on a rollback journal (a creation stopped before its `PRAGMA journal_mode = WAL`, or an operator's choice) could not be indexed or searched, an uncoded `attempt to write a readonly database` and a 0-byte index.db, because the index's journal pragma reached the store it attaches read-only; `item export --output` created its file `0644`.

## What landed

- src/client/reader.rs: `snapshot`, a read transaction around the overlay listings and `count_items`, none inside an open one.
- src/client/search.rs: `PRAGMA main.journal_mode = WAL`, the index's alone.
- src/cli/item.rs: `item export --output` creates its file `0600`.
- Tests: tests/search.rs `a_store_on_a_rollback_journal_is_indexed` (failing on dcf49e2). The snapshot needs a writer committing between two statements and is judged by reading; the export mode was not tested.
- Left: the cutoff validation of `evict_before` and `release_before` (no caller; a new store error code), and setting WAL before the creation transaction, which leaves `auto_vacuum` unset (checked with sqlite3 3.53): the journal mode stays chosen once, after creation.

## Capabilities moved

store ("A reader answers from one snapshot"), search ("The index sets its own journal mode alone"), cli ("An exported body is the store's user's alone"), all added.

In the working tree on io-pimdir dcf49e2 (0.15.1), uncommitted, no version bumped; CHANGELOG.md [Unreleased] names it.
