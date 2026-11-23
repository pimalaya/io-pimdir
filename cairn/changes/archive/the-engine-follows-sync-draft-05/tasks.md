---
cairn: tasks
change: the-engine-follows-sync-draft-05
---

- [x] Harness: `failed`, `parked`, `accept_absences`, `blob` false, `corrupt`, `if_match` on `Edit`, events of every verb, rows compared on the columns each expected row names, every failing vector named at once
- [x] Vector 92: a handle-less accepted add stays pending, no `Created` (sync-audit S11)
- [x] Vector 93: a member disagreeing on subject, date or sender with a held `Message-ID` is minted, in the page's naming and in the write (tests/another_message.rs failed first: joined `basic-1@example.org`)
- [x] Vector 94: a body failing its name is never pushed (`WantsVerify`, `PimdirSyncReport::unreadable`)
- [x] Vectors 87, 88: `PimdirMutation::Edit { if_match }`, an unreadable `Add` body fails the run
- [x] Vector 79: the run fails, the round left at its first page
- [x] Events tied to commit (failed first: "reported before its write: [Added]")
- [x] S13: an equal carried body settles rather than conflicts (failed first: one conflict)
- [x] S15: the flag axis with no base is the union (failed first: no push, the local flag dropped)
- [x] Level 0 in `item_from_row`
- [x] `PimdirLinkId::stated`, seam docs on `Add` and `Remove`
- [x] Vectors 76 and 93 reported to pimdir as contradicting SYNC §5 and §9; pimdir fixed both (its log 2026-10-30-two-sync-vectors-follow-the-prose), and the harness runs them as written
- [x] Spec: sync.md, seam.md, mutate.md; fold, log, CHANGELOG
