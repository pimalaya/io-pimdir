---
cairn: delta
change: a-page-without-a-cursor-is-refused
---

## ADDED Requirements

### Requirement: A page that cannot be resumed past is refused
A round page (`complete`) that is neither `last` nor carries a cursor SHALL complete the sync with `PimdirArgError::UnresumablePage`, writing nothing of the page: read as the last, it would find absent every member the round has not listed (pimdir SYNC §4). What earlier pages landed stays, the round open.

#### Scenario: A connector dropping its cursor
- GIVEN two synced bindings and a round page listing one of them with `last` false and no cursor
- WHEN the sync takes the page
- THEN it completes with `UnresumablePage` and writes nothing, the other binding kept (src/sync.rs `a_page_neither_last_nor_cursored_is_refused`)

## MODIFIED Requirements

### Requirement: A rebuild reads every page before its one batch
Appended: a page neither `last` nor carrying a cursor SHALL complete the rekey with `PimdirArgError::UnresumablePage`, and a page that is not a round's (`complete` false) with `PimdirArgError::IncompleteListing`, writing nothing: the rebuild's absence covers every member, so only a whole round may answer it.

#### Scenario: A delta answering a rekey
- GIVEN an old spine of two placements
- WHEN the rekey's round request is answered by a delta listing one member
- THEN it completes with `IncompleteListing` and writes nothing (src/rekey.rs `a_delta_answering_a_rekey_is_refused`)
