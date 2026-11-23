---
cairn: change
id: the-engine-follows-sync-draft-05
status: landed
created: 2026-10-30
---

# The engine follows sync-draft-05

## Why

pimdir landed sync-draft-05 on 2026-10-30 (its cairn/log/2026-10-30-should-fix-before-the-freeze.md, a-merge-keeps-every-body.md, a-read-body-is-verified.md). Beside the merge (the-agreed-body-stays-pinned), the seam (a-queued-edit-names-what-it-changes) and completeness (a-listing-proves-what-it-drops), it binds the engine to rules io-pimdir breaks today, each guarded by a vector tests/vectors_sync.rs now runs:

- an add accepted with no handle (IMAP without UIDPLUS) drops its provisional binding and reports `Created` for a member nothing names (vector 92; sync-audit S11);
- a second source's member stating a held `Message-ID` whose summary names another message joins the held item (vector 93);
- a pending create whose body fails its name is pushed (vector 94);
- an `Add` whose body cannot be read, and an `Edit` stating the body it replaced (`if_match`) once another write replaced it, are engine rules the harness did not run (vectors 87, 88);
- a run failing on a page neither last nor carrying a cursor is a failed run the harness did not compare (vector 79);
- an event is reported only once the write recording it committed (SYNC §5): the engine reports the events of a write as it derives them, before the write is serviced.

## What

- Sync: a handle-less accepted add stays pending, unreported, not pushed again in the run; a page joins a held `Message-ID` only when the summary agrees on subject, date and sender, minting otherwise; every body a chunk pushes (`Add`, `Update`) is verified against its name through a new storage yield before the chunk goes, a body failing it withheld and counted (`PimdirSyncReport::unreadable`); events buffered per write and moved to the report when the write is answered.
- Mutate: `PimdirMutation::Edit` gains `if_match: Option<PimdirHash>`, refused as `PimdirMutateError::Replaced` when the placement holds another body.
- Harness: `failed`, `blob` false and `corrupt` objects, `if_match` on an `Edit`, events of every verb.
- The audit's S13 (an equal carried body still conflicts) and S15 (a flag axis with no base drops local-only flags), each with a test failing first.
- Level 0 in `item_from_row` (src/client/write.rs): storage-draft-05 closes `level` to 1 and 2 and the reconcile repairs a 0 (`repair_levels`), so the owner's read no longer special-cases it.
- A connector strips one leading `esc:` before naming a stated identity to its server (STORAGE §9): seam documentation only, the engine names no identity to a server but the `link_id` of an `Add` and a `Remove`, which stays the store's key.
