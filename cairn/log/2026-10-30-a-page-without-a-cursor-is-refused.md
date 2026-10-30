---
cairn: log
change: a-page-without-a-cursor-is-refused
date: 2026-10-30
---

# A page without a cursor is refused

A round page with `last` false and no cursor closed the round, dropping every unlisted binding in scope as deleted, which the hub then pushed to every other source; a rekey did the same, and took a delta answering it as the whole collection. Both are now refused before anything of the page is written (pimdir SYNC §4, §8; spec edit 3 of 2026-10-30).

## What landed

- src/coroutine.rs: `PimdirArgError::UnresumablePage` and `PimdirArgError::IncompleteListing`.
- src/sync.rs: a round page neither last nor cursored completes the sync with `UnresumablePage`; a last page carrying a cursor still closes the round.
- src/rekey.rs: a page that is not a round's completes the rekey with `IncompleteListing`, one neither last nor cursored with `UnresumablePage`.
- Tests: src/sync.rs `a_page_neither_last_nor_cursored_is_refused`, src/rekey.rs `a_page_neither_last_nor_cursored_is_refused` and `a_delta_answering_a_rekey_is_refused`, each failing on 4c3353e with a write dropping the unlisted placement as `Deleted`.
- Capabilities moved: sync ("A page that cannot be resumed past is refused", added), rekey ("A rebuild reads every page before its one batch", appended).
