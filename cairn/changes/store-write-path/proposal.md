---
cairn: change
id: store-write-path
status: landed
created: 2026-08-25
---

# Three places the store pays a whole-store cost for a one-row question

## Why

Each of these is the shape the `store-algorithm-audit` fixed elsewhere, left in three places it did not reach.

- **The writer lock is held across the blob write.** `apply_ops` writes a body between `BEGIN IMMEDIATE` and `COMMIT`, so SQLite's single writer lock spans a file write, two `fsync`s and a rename. pimdir SPEC §14 says the opposite, in as many words: the blob write MAY happen before `BEGIN` and an implementation writing bodies of any size SHOULD do so, because inside the transaction it serialises every other writer behind an I/O path that touches no database page.

- **The collector holds every hash in memory.** `collect_garbage` reads `LIST_OBJECT_HASHES` into a `BTreeSet<String>` to diff the blob tree against, which is hundreds of thousands of 52-character names at the scale §1 promises, to answer a question that is always about one file.

- **A purge reads every row it is about to delete.** `purge` and `purge_retained_before` select the pinned hashes and then delete the same rows: two passes over the same set, and the sweep is the one that takes fifty thousand of them at once.

A fourth, found while reading: `LIST_GARBAGE_OBJECTS` says `refcount = 0` where the canonical statement says `<= 0`. It has no caller, so nothing was wrong, but it is the statement a consumer reaches by name and it no longer matches the partial index built for it.

## What

- Bodies are staged before the transaction opens (`stage_blobs`), in `write` and `write_rekeyed`. `apply_ops` keeps its own write as the floor for the queue drain, which builds its ops inside the transaction that claims the row; the write is idempotent, so that costs one `exists` check.
- The collector asks `OBJECT_EXISTS` per file, on the primary key, and the format gains that statement.
- `PURGE_ITEM` and `PURGE_RETAINED_BEFORE` become `DELETE ... RETURNING object_hash, conflict_object`, and `RETAINED_ITEM_BY_SEQ` and `RETAINED_BEFORE` are retired.
- `LIST_GARBAGE_OBJECTS` matches the canonical text.
