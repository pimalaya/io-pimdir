---
cairn: change
id: the-index-reads-the-canonical-columns
status: landed
created: 2026-10-30
---

# The index reads the canonical columns

## Why

pimdir landed four of this crate's search gaps in search-draft-05 (its log search-gaps-io-pimdir-found, 2026-10-30), each moving a private mechanism of this crate into the canonical schema and statements:

- SEARCH §2: an index whose shape differs from migrations/search/0001_init.sql is rebuilt; the shape, not a private build number, tells an index an earlier draft built.
- SEARCH §4: `index_meta.store_reconciled`, written by `set_index_cursor` and answered by `load_index_meta`, in place of this crate's `index_build.reconciled`.
- SEARCH §6, §8: `object.attachment`, written by `insert_object`, read by `attached`, in place of the `pimdir_column_size` function decoding `object_text_docsize`.
- SEARCH §8.2: `behind`, the canonical statement for the lag this crate computed in Rust.

The vendored spec/ no longer matches pimdir (tests/spec_drift.rs fails: `ATTACHED` has no constant).

## What

- Re-vendor migrations/ and queries/ from pimdir byte for byte; build.rs generates `ATTACHED`, `BEHIND` and the changed `INSERT_OBJECT`, `SET_INDEX_CURSOR`, `LOAD_INDEX_META` from them.
- `has:attachment` on a calendar item embeds `attached`; `insert_object` binds `:attachment` (1 when the extraction wrote text into the field); the docsize function and its registration go.
- The coverage's `behind` runs `behind`.
- The refresh compares collection stamps with `index_meta.store_reconciled`, written by `set_index_cursor` (a batch passing the stored value, a completing pass the cursor read before it); `index_build`, `BUILD`, `SET_BUILD`, `HAS_BUILD`, `LOAD_BUILD`, `LOAD_RECONCILED`, `SET_RECONCILED` go.
- An index is current when its version, tokenizer and shape match: every table, index and trigger 0001_init.sql creates, compared by `sqlite_schema` text with comments removed and whitespace collapsed, FTS5's shadow tables and any other table left out. `open` rebuilds any other; `open_query` refuses it as `IndexOutdated`.
