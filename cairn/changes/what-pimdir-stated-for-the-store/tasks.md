---
cairn: tasks
change: what-pimdir-stated-for-the-store
---

- [x] 1. Test: an unbound `update` whose `if_match` names another body parks (passed before: `stage_action` checks the fence before the item is found unbound; pimdir's reading of `apply_unbound` not reproduced)
- [x] 2. Test on the fixtures: a source lacking the coverage columns reads as filling, never complete (failing first: 0.5.1 listed no source)
- [x] 2. Test on the fixtures: every public read of a reader and a producer answers, nothing written; `categories` reads as its default `'[]'` (src/client/schema.rs)
- [x] 2. `schema::as_reconciled` for the reader's statements over what an older store lacks; the guards rewriting statement text removed (the reads of a wholly missing table keep answering no row up front)
- [x] 3. Test: the reconciliation drops another implementation's table, column, index and trigger (passed before: already the behaviour)
- [x] 3. No io-pimdir state in pimdir.db (checked)
- [x] Spec: store.md folded; log; CHANGELOG
