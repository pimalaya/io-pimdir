---
cairn: log
change: item-sort-key
date: 2026-08-08
landed: 2026-08-08
---

# A collection can be ordered, and renamed

`items.sort_key`, the `items_by_sort` index, `list_items_page_asc` and
`list_items_page_desc` with their `(sort_key, seq)` cursor, `set_sort_key`, and
`rename_collection` with `ON UPDATE CASCADE` behind it. All of it mirrors the
specification, which moved first.

## The state this landed into

The crate was already **behind its own spec**, and nothing said so. Its inlined
`MIGRATION_0001` had no `sort_key` column and no `ON UPDATE CASCADE` on any key,
so it was creating stores that did not match the format it claims to implement:
a reader expecting an ordering column would have found none, and a rename would
have been refused or, with foreign keys off, orphaned an entire collection.

That is the interesting failure, not the missing feature. `sql` exists to be
*the* canonical copy, handed to Pimalaya Android by name across a JNI boundary
precisely so nobody transcribes it. A canonical copy that silently disagrees is
worse than no copy, because consumers stop checking.

So the first thing built was `tests/spec_drift.rs`, and it earned its place
immediately: it confirmed the schema mirror was right, and then found two
statements missing from `sql::ALL` that the eye had passed over.

## What the drift test compares, and what it deliberately does not

- **The schema, semantically**, through SQLite's own pragmas after applying
  both: tables, columns with their types, nullability, defaults and primary-key
  positions, foreign keys **with their `ON UPDATE` and `ON DELETE` actions**, and
  declared indexes. Not by text, so prose and formatting are free to differ while
  a dropped default is still caught.
- **The statement set**, by name. An implementation may not quietly drop an
  operation.
- **Not statement text.** SPEC §4.4 permits an equivalent substitution, and this
  crate uses that permission in three places. Requiring textual equality would
  forbid what the specification allows.

The two the test caught, `DELETE_ITEMS` and `RECOMPUTE_REFCOUNTS`, turned out to
be legitimate: this crate saves by diff rather than replace-all, and repairs
refcounts by per-hash net change rather than a full recompute. Both are offered
by the spec as alternatives. They are now in a `SUBSTITUTED` list that names
what replaced each, which is the difference between a documented substitution
and an accidental omission. Nothing else may go missing without failing.

## The diffed save turned out to be the invariant

§9.3 requires that an ordinary write preserve an item's key. The reference save
is a replace-all, which is why the spec has `load_items` carry `sort_key` back
out. This crate never needed that: it inserts new items and updates existing ones
in place, and `UPDATE_ITEM` names no `sort_key`, so a key survives by never being
touched. `LOAD_ITEMS` therefore stays as it was and now says why it differs.

Pinned by a test that writes a placement, sets its key, re-writes the same
placement as a re-sync would, and asserts the key is still there.

## Renaming

The cascade is on two keys, not one. `collections(id)` is the obvious parent;
`items(collection, link_id)` is also one, of `bindings`, and cascading the first
makes `items.collection` change, which the second refuses under the default
`NO ACTION`. The test renames a synced collection and asserts the binding
followed, not just the item: without the binding the next sync treats every item
as new and re-downloads the collection, which is the silent version of the bug.

## Not done

io-replica still carries no sort key on `ReplicaPlacement`, so nothing writes one
through the ordinary path yet. `set_sort_key` is how a consumer populates keys in
the meantime, from the `meta` it wrote itself. That field is io-replica's own
change.

Capabilities moved: **store** (ordered paging, the write-preserves invariant,
rename), **spec-fidelity** (added).
