---
cairn: log
change: a-narrowed-round-proves-only-what-its-enumeration-lacks
landed: 2026-11-15
---

# A narrowed round proves only what its enumeration lacks

Follows pimdir's change of the same id (status draft there, the owner's decision open, its recommended reading in both trees), in the working tree of both repositories, uncommitted. A narrowed round found absent any member dated before `round_started_at`, whatever its enumeration: live, a band narrowed by arrival dropped 13 Gmail and 24 Graph messages dated 2026-01-01 and received in November, which the enumeration listed. spec/ is re-vendored from the pimdir working tree; spec/REVISION still names 83102f0, so tests/spec_drift.rs reports the nine re-vendored files until pimdir's change is committed and the revision follows.

## What moved

- `list_unstamped_bindings` (vendored): a narrowed round's absence is only what `enumerated` lacks, none without it.
- src/sync.rs: the last page loads `LastPage { narrowed: false, enumerated: None }` and applies the page's proof itself, for a round that page opens as for one opened before; a narrowed page handing no enumeration finds nothing absent and counts what it left unlisted in `PimdirSyncReport::unlisted`, as a capped page does; `stamped` reads every stamp, so a resumed round whose earlier page named a member the enumeration holds still closes; `State::Opening`, the write opening a narrowed round before its last page so the store stamped the instant, is gone.
- Docs: `PimdirRemoteSnapshot::narrowed` and `enumerated`, `PimdirLoadSelection::LastPage`, `PimdirSyncReport::unlisted`, the sync module's header.

## Checks

- Vectors 107 (rewritten), 160 and 161 (new) failed with the old statement baked in, the runner reporting their events: 107 dropped 14, 160 dropped 9 (the CI message), 161 dropped 13 (the `Date` ahead of its arrival); they pass now.
- tests/scoped_sync.rs `a_resumed_narrowed_round_handing_no_enumeration_drops_nothing`: on the old engine and statement it parked (`PimdirParked { absent: 1, bound: 3 }`); now it closes, drops nothing and reports three unlisted. `a_resumed_narrowed_round_drops_only_what_its_enumeration_lacks` holds the stamps the empty-listing guard reads: the load is unbound so they show, a stamp on a member the enumeration holds included, which the statement bound with the page's narrowing would leave out.
- Capabilities moved: sync ("A narrowed listing proves only what its enumeration lacks", replacing "… what its filter could list"; "Absence means deleted in scope only"; "A capped listing proves no absence"), seam ("A narrowed listing says so").
