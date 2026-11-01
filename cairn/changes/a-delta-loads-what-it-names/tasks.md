---
cairn: tasks
change: a-delta-loads-what-it-names
---

From the 2026-10-11 performance audit (p1p2-report #2), re-verified on 2026-10-18 at src/sync.rs:589.

- [ ] Decide how "owes a push" is answered (proposal.md): a `Pending` scope over the change feed plus a marker for what an earlier run left pending, or an index over bindings differing from their base
- [ ] pimdir: SYNC §10 (the scopes, the steps reasoning about absence), STORAGE §14 and the statement answering the new scope, a sync vector, checks/invariants.sh (its plan seeks)
- [ ] `PimdirLoadSelection`: the new scope, or `Handles` and `Links` together, in src/load.rs; the store's answer in src/client/write.rs
- [ ] src/sync.rs: a delta and a last page load the listed and vanished handles, the hints' links and the pending placements; the last page takes absence from `unstamped`
- [ ] Test: every engine suite and vectors/sync unchanged; a delta over a large collection loads a bounded set (a load-count assertion, as the android bench measured)
- [ ] Bench: a 50-message step into a 100k and a 1M collection, before and after
- [ ] Fold seam.md, log, CHANGELOG; android drops nothing (it already passes the scope through)
