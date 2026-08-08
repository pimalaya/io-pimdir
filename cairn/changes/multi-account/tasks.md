---
cairn: tasks
change: multi-account
---

# Tasks

- [x] `sql`: `collections.account` plus the partial `collections_by_account`
      index, in place at version 1; `ENSURE_COLLECTION` and
      `SET_COLLECTION_KIND` binding `:account`; `SET_COLLECTION_ACCOUNT` and
      `LOAD_ACCOUNT`; `LIST_COLLECTIONS` returning it, plus
      `LIST_COLLECTIONS_BY_ACCOUNT` and `LIST_ACCOUNTS`; the two multiplicity
      reads. Kept structurally identical to `pimdir/migrations/0001_init.sql`.
- [x] `client`: `PimdirStore.account` with the `for_account` builder and the
      `account` getter, bound at every collection-creating call site (the
      deliberate `ensure_collection`, the lazy one in the write seam, the
      producer's enqueue).
- [x] `client`: `PimdirCollection.account`; `list_collections_by_account`,
      `list_accounts`, `set_collection_account`, `collection_account`.
- [x] `client`: `PimdirPlacement` plus `link_placements` and
      `object_placements`.
- [x] `reconcile_draft_shape`: heal a store written by an earlier draft of
      version 1 (add the column and the index if missing), so an existing store
      opens rather than failing on a missing column.
- [x] Tests: the seq stays shared across accounts; a body reaching two accounts
      is one object; the two multiplicity reads; by-account listing including
      the `NULL` case; regrouping disturbs no identifier.
- [x] `pimdir` spec repo: SPEC.md §9.2, the terminology entry, §4.3, §12.1, the
      migration and the two query files. **Done first**, since the spec is the
      source of truth this crate inlines.
- [x] Fold the delta into `cairn/spec/store.md` and write the log entry.
