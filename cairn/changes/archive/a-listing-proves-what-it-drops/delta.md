---
cairn: delta
change: a-listing-proves-what-it-drops
---

## ADDED Requirements

### Requirement: A mass absence parks the round
A round's last page finding absent more than half of the based bindings of the source in the round's scope, counted as the deletes are, SHALL NOT close the round when those bindings number at least 20 (`PimdirParked::MASS_ABSENCE`), unless the run carries `PimdirSyncOptions::accept_absences` (pimdir SYNC §5, vectors/sync/77, 78). It drops nothing and writes no `CloseRound`: the page lands as a page before the last, its members named and stamped, its checkpoint recorded with no cursor, and `PimdirSyncReport::parked` carries the absent and the bound counts. A round that held a cursor is drawn anew in that write, so the next run lists it from its first page and decides again. `accept_absences` is per run and never stored.

### Requirement: A delta page is the last
A delta page (not `complete`) that is not `last` SHALL complete the sync with `PimdirArgError::UnresumablePage`, writing nothing of it: a delta is one page, always the last, and its checkpoint would claim the changes the pages it says follow list (pimdir SYNC §4; sync-audit S12).

#### Scenario: A paged delta
- GIVEN a delta page with `last` false, a cursor and a checkpoint
- WHEN the sync takes it
- THEN it completes with `UnresumablePage` and lands no checkpoint (src/sync.rs `a_delta_page_not_last_is_refused`)

### Requirement: A rekey missing most members parks
A rekey whose listing leaves more than half of the source's based bindings in the collection with no member, those bindings numbering at least 20, SHALL write nothing (no drop, no upsert, the checkpoint and the generation unmoved) unless `PimdirRekey::accept_absences` is set (`PimdirSourceStore::rekey(collection, accept_absences, remote)`), and `PimdirRekeyReport::parked` carries the counts (pimdir SYNC §8, vectors/sync/89). The old checkpoint still names the old handle space, so the next run names the rekey again.

### Requirement: A server collection is deleted only when enabled
An engine mirroring collections between sources SHALL NOT delete a collection on one source's server because another source stopped listing it unless its configuration enables it, and then only when that collection is empty or every member was reconciled in the run (pimdir SYNC §5). The library mirrors no collection: the rule is the connector's and its configuration's, documented on the seam; `delete_collection` refuses a collection holding queued actions (the-store-follows-storage-draft-05).
