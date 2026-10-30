---
cairn: tasks
change: a-page-without-a-cursor-is-refused
---

- [x] Test: a round page with `last` false and no cursor completes the sync with `UnresumablePage`, no write (fails on 4c3353e: the round closes and drops the unlisted binding)
- [x] Test: the same page to a rekey completes with `UnresumablePage`; a delta answering a rekey completes with `IncompleteListing` (both fail on 4c3353e)
- [x] src/coroutine.rs: the two `PimdirArgError` variants
- [x] src/sync.rs, src/rekey.rs: refuse the page
- [x] Spec: sync.md, rekey.md; fold, log, CHANGELOG

The pimdir vector for the refusal (spec edit 3) is run under a-listing-proves-what-it-drops once it lands.
