---
cairn: log
change: a-listing-proves-what-it-drops
date: 2026-10-30
---

# A listing proves what it drops

pimdir sync-draft-05's completeness (absence-deletes-only-from-a-complete-round, a-rekey-parks-a-mass-absence): only a closed complete round's absence in scope deletes, a mass absence parks, a delta is one page.

## What landed

- src/sync.rs: a round's last page finding more than half of at least 20 based bindings in scope absent parks (`PageKind::Parked`): no drop, no `CloseRound`, members named and stamped, the checkpoint landed with no cursor, a round that held a cursor drawn anew so the next run lists it from its first page; `PimdirSyncReport::parked`, `PimdirSyncOptions::accept_absences`, `sync::PimdirParked`. A delta page that is not the last is refused with `UnresumablePage` before its write (S12).
- src/rekey.rs: the same guard over the based bindings the rebuild would drop `Deleted`, nothing written when it parks; `PimdirRekey::accept_absences`, `PimdirRekeyReport::parked`, `PimdirSourceStore::rekey(collection, accept_absences, remote)`.
- src/remote.rs: a connector deletes a server collection another source stopped listing only when enabled, and only empty or fully reconciled (SYNC §5).
- Failing first: vectors 77 ("events differ"), 89 ("bindings rows differ"), 79 (the harness read the refusal as a panic rather than a failed run); src/sync.rs `a_delta_page_not_last_is_refused` (the page's `SetCheckpoint` landed). Vectors 74 and 75 reproduce with the harness comparing each row on its own columns; 76 carries an erratum (the-engine-follows-sync-draft-05).
- Capabilities moved: sync ("A mass absence parks the round", "A delta page is the last", added), rekey ("A rekey missing most members parks", added), seam ("A server collection is deleted only when enabled", added).
