---
cairn: change
id: the-crate-follows-draft-14
status: landed
created: 2026-11-18
---

# The crate follows draft-14

## Why

pimdir 466dda6 bumps the three parts to storage-draft-14, sync-draft-14 and search-draft-14, carrying cairn/changes/the-draft-14-lines there: the refusal exceptions (D34), a tombstone outranking a conflict (PIM-12), a move in place refused (PIM-14), a refused offered create bound with its refusal (D6), and the order of format versions (C1, vectors/versions.json).

## What

- spec/ re-vendored at pimdir 466dda6, spec/REVISION naming it.
- The version constants and the version tests move to draft-14 (earlier draft-13, later draft-15).
- The engine binds an offered create on its refusal; `PimdirPushOutcome` documents the exceptions of D34; tests/versions.rs runs vectors/versions.json.
- PIM-12 and PIM-14 needed no code: the hub projected a deleted row `Tombstone` already, and `move-in-place` landed on 2026-11-17.
