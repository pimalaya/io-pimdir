---
cairn: change
id: format-conformance
status: landed
created: 2026-08-25
---

# The format moved and half of it arrived; nothing was watching

## Why

pimdir's `retained-page-by-seq` moved the trash listing onto the public `seq` and re-ordered `items_retained` to match. This crate took the statement and left the index, so every trash page sorted every retained row of the collection to return fifty: the exact regression that change was written to remove, reintroduced by taking half of it. The pimdir log even says whose entry the other half was.

`spec_drift` caught it, and nothing ran `spec_drift`: this repository has no CI, and the suite skips silently when the pimdir checkout is absent. So does `conventions`. And `objects.json`, the one vector file the format makes a **MUST**, was read by nobody at all, while the `SHOULD` one was checked.

Three of the four things that bind this crate to the format were therefore unenforced, and the fourth (`every_canonical_statement_is_inlined`) checks names and never loads the SQL, so a spec edit naming a column the migration lacks would reach a consumer before a test.

## What

- `items_retained` is `(collection, seq)`, and `RESHAPED_INDEXES` drops an index whose columns moved before the ensure batch runs: `CREATE INDEX IF NOT EXISTS` keys on the name, so it silently leaves an old shape in place.
- `tests/objects.rs` checks the MUST vector: every body's name under both algorithms, its shard path, and the same names again through the streamed hasher.
- `spec_drift` gains `every_canonical_statement_prepares`, against the canonical schema. This is the only place the format's own SQL is ever loaded.
- `PimdirBlobs::path` is public, because the shard path is normative and §14 invites a consumer to stream a body straight to it.
- CI, in both repositories, each asserting the spec suites ran rather than skipped.
