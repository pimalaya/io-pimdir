---
cairn: tasks
change: a-page-lands-in-few-statements
---

# Tasks

## 1. Measure (android)

- [x] Debug-only profile: time per canonical statement name inside `PimdirStorage.applyWrites`, summed per batch, appended to the slow-write log line
- [x] Ignored Robolectric test (FillPageMeasure, gated on `PIMALAYA_MEASURE`): 500 new mail upserts (two flags, a summary with addresses each) through `applyWrites` into a fresh store, timed with the schema as is and with the stamp triggers dropped, profile printed
- [x] Device run on the Microsoft test account (the user runs it): the profile of a fill page from the log (closed 2026-11-19, the change abandoned: see proposal.md, Closed)
- [x] Record the desktop numbers in this proposal
- [x] Record the device numbers; decide steps 2 and 3 (closed 2026-11-19, the change abandoned: see proposal.md, Closed)

## 2. One stamp per batch (pimdir first)

- [x] pimdir: the pending stamp in STORAGE §4.5 and §17, the trigger guards in migrations/storage, `stamp_pending` in queries/storage, a check for pending rows at rest, vectors; same id, landed first (closed 2026-11-19, the change abandoned: see proposal.md, Closed)
- [x] Re-vendor spec/, bump REVISION; `client::write::apply` writes new items pending and runs `stamp_pending` before commit (closed 2026-11-19, the change abandoned: see proposal.md, Closed)
- [x] Android: `applyUpsert` writes new items pending, `applyWrites` runs `stamp_pending`; device run, a page of 500 under 0.5 s in the log (closed 2026-11-19, the change abandoned: see proposal.md, Closed)

## 3. Fewer statements per message (the desktop says step 2 alone misses the target)

- [x] The sender rule once per page over its links (closed 2026-11-19, the change abandoned: see proposal.md, Closed)
- [x] The list binding: `create_param` only when `clear_param` fails (it ran before every bind and slowed every statement of the connection), retain and add flags bound once (`PimdirDb.changedSharing`); host page of 500 from 355 to 106 ms (2026-11-05)
- [x] Device numbers after the list-binding fix; re-decide steps 2 and 3 from them (closed 2026-11-19, the change abandoned: see proposal.md, Closed)
- [x] One chunked lookup by link for unheld members, replacing `applyUpsert`'s per-message reads (closed 2026-11-19, the change abandoned: see proposal.md, Closed)

## Close

- [x] Fold into cairn/spec/store.md, log, CHANGELOG (here, pimdir, android) (closed 2026-11-19, the change abandoned: see proposal.md, Closed)
