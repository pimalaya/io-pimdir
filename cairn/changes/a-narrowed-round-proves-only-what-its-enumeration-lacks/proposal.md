---
cairn: change
id: a-narrowed-round-proves-only-what-its-enumeration-lacks
status: active
created: 2026-11-15
---

# A narrowed round proves only what its enumeration lacks

## Why

Follows pimdir's change of the same id (pimdir cairn/changes/a-narrowed-round-proves-only-what-its-enumeration-lacks, status draft, the owner's decision open). The live campaign of 2026-11-15 lost 13 Gmail and 24 Graph messages from a replica: a widening band round narrowed by a date filter reading the arrival did not list messages dated 2026-01-01 and received in November, and `list_unstamped_bindings` found them absent because they were dated before `round_started_at`, though the last page's enumeration listed them. This crate vendors that statement, and the Android app runs it through this crate's engine.

## What

1. **Re-vendor** spec/ from the pimdir working tree of 2026-11-15: the new `list_unstamped_bindings` (a narrowed round's absence is only what `enumerated` lacks, none without it), vectors 107 (rewritten), 160 and 161 (new), 47's note, the manifest and the README, and `load_item_capabilities` of the other change of the day.
2. **The engine reads every unstamped binding of the scope** at the last page (`PimdirLoadSelection::LastPage { narrowed: false, enumerated: None }`) and applies the page's proof itself, for a round that page opens as for one opened before: a capped page and a narrowed page handing no enumeration find nothing absent and count what they left unlisted in `PimdirSyncReport::unlisted`; a narrowed page handing one finds absent what it lacks.
3. **The empty-listing guard stays exact.** Bound with the page's narrowing, the statement now leaves out every member the enumeration holds, so `stamped`, which tells a resumed round that named a member by the bindings the store does not answer unstamped, could not tell such a member stamped from one the enumeration merely holds, and would park a round only its earlier pages named, or close one that named nothing. Read unbound, every stamp shows.
4. **No opening write of its own.** A narrowed round its last page opens was opened in a write before that page's load so the store stamped the instant the old rule read; nothing reads it for an absence now, so the round opens in its page's write as any other, and `State::Opening` goes.
5. Docs: `PimdirRemoteSnapshot::narrowed` and `enumerated`, `PimdirLoadSelection::LastPage`, `PimdirSyncReport::unlisted`, the sync module's header; cairn/spec/sync.md and seam.md.

## Decided

- **The engine applies the proof, the statement keeps its parameters.** `LastPage`'s `narrowed` and `enumerated` stay, a storage binding them as the canonical statement says, so the API does not break (the Android app's Java storage reads them over its wire); the engine binds neither. A storage still running the old statement then answers the engine correctly too, since unbound the old and the new text agree.

## Lands in

The working tree of 2026-11-15, uncommitted, spec/REVISION still naming pimdir 83102f0 until pimdir's change is committed, so tests/spec_drift.rs reports the re-vendored files until then.
