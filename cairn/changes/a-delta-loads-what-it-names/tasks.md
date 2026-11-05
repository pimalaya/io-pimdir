---
cairn: tasks
change: a-delta-loads-what-it-names
---

From the 2026-10-11 performance audit (p1p2-report #2), re-verified on 2026-10-18 at src/sync.rs:589.

- [x] Decide how "owes a push" is answered (proposal.md): neither a `Pending` scope nor a writer-kept index, but two statements over the bindings and items, `list_owing_bindings` and `list_lacking_items`, pimdir storage-draft-09 (cairn/changes/storage-draft-09 there)
- [x] pimdir: SYNC §10, STORAGE §14 and the two statements, checks/invariants.sh running them against every status and planning the seek
- [x] `PimdirLoadSelection::Owing` in src/load.rs; the store's answer in src/client/write.rs
- [x] src/sync.rs: a delta loads `Owing` of the listed and vanished handles, then its hints' `Links`
- [ ] A round's last page keeps `All`: its absence and its park are counted over every based binding of its scope (pimdir SYNC §5); a count of those by statement would let it load `Owing` too
- [x] Test: every engine suite and vectors/sync unchanged; src/sync.rs `a_delta_loads_what_owes_a_push_never_the_collection` (fails on `All`), tests/owing_load.rs
- [ ] Bench: a 50-message step into a 100k and a 1M collection, before and after (the Android phone run of pimdir cairn/changes/performance-audit-2026-11-05 measures the app's)
- [x] Fold seam.md, log, CHANGELOG; android passes the selection through
