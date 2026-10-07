---
cairn: tasks
change: scoped-mail-sync
---

- [x] Re-vendor spec/ from pimdir `a2e92af`; reconcile `sources`, `bindings.round`, `sources_stamp_coverage` and the dropped `probes` on open.
- [x] Seam: `PimdirRemoteMeta` on `PimdirRemoteItem`; `PimdirRemoteSnapshot` pages (`last`, `cursor`, optional `checkpoint`); `PimdirEnumerate`, `PimdirListing`, `PimdirEnumerated`; `PimdirRemote::scope_bound`; `PimdirScope`, `PimdirCoverage`, `PimdirRound`, `PimdirCursor`; the round write ops.
- [x] Engine: listing chosen from coverage and round; pages named and merged, stamped, cursor landed; deletes at the last page in scope only; creates wait for the page, out-of-scope ones for a round holding them; pulls take the member's meta and carried body; probes and `Probed` gone.
- [x] Upgrade revisits unheld claims only and keeps a walked mark; rekey lists every page, names from the meta, fetches nothing.
- [x] Store: load answers coverage, round and unstamped bindings; writes apply the round ops, stamps after the upserts, public ids in batch order; unnamed upserts refused; level 0 read as an unheld claim.
- [x] Readers and owner: coverage on collections, `list_coverage`, mail counts, filtered pages, search, `collect_before`; CLI `collection list` shows the coverage.
- [x] Annex A.1: `derive_meta`, `meta_attachment`; tests/summaries.rs checks `meta_attachment`.
- [x] tests/vectors_sync.rs: summaries, rounds and coverage seeded and compared, pages, `interrupted_after`, `cursor_rejected`, `scope_bound`; all 45 vectors green. tests/scoped_sync.rs for the store side.
- [x] Fold into cairn/spec (sync, upgrade, rekey, seam, store, hub, mutate, summaries, cli); log; CHANGELOG.
