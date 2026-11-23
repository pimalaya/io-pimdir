# Report, the io-pimdir lane, 2026-11-23

For the owner. Times and decisions in journal.md, each app request in answers.md, hashes in drops.md.

## Losses first

- R4 and R1, the rekey's two silent losses (pimdir cairn/changes/a-smaller-sync), are fixed in drop 2: an owed flag change survives a listing with unknown flags (vector 201), a binding's conflict survives a listing with no revision (202).
- No new loss found. One more defect of that family showed at drop 2's gate (a rekey leaving a conflict's diverging body empty though the listing carried it); fixed in drop 2, pinned by tests/engine/rekey_cases.rs.

## What landed

- Drop 1 (cc3021b): CI green (Windows, per-feature builds); `sql::search` under `indexer` for the Android app.
- Drop 2 (2314084; pimdir 0401aab, 2f51d96): the rekey follows SYNC §8 in all nine K3 cases (vectors 201 to 213, sync-draft-15 edited in place). Rule D10 lands a pending create on its member, so neverest's renumbered INBOX sends no second APPEND. The drain joins another source's item (K4). An undecodable pending row is listed opaque instead of failing the listing (G1, signature kept). `summary::DERIVATION` is public.
- Drop 3 (17d4ea8; pimdir 087410f pins it): 0.20.0 in Cargo.toml and nix; CHANGELOG `[0.20.0] - 2026-11-23` with the start-over note and the consumers' feature lines; MSRV 1.89 restored (drop 2 had broken it); himalaya N6 (hit columns read under `search` alone) and N7 (`accounts_of` without the coverage join); 72 change folders archived.

## Checks

- Oracle: 131f9bf against drop 2 over 100,000 scenarios of every weight, 14,157 differences, all expected (a fixed case's trace line fired in the run), 0 unexpected. Drop 3 against drop 2: 0 differences.
- Perf, 517k store copy, CPU 15, measured while other lanes built: no-op sync 86.6/85.6 ms (7f16b27) against 87.1/86.6 (RC), A/A 1.1% apart; no-op skipping settled folders 33.0/33.4 against 32.8/32.8; 300 new, re-run interleaved, medians 227.7/218.0 against 157.8/222.9 (disk-bound noise, user CPU equal); rekey_scale at 8,000 members, medians of six: coroutine 37.95 against 37.9 ms, write 179.2 against 178.9. None past the 3% gate. queries.sh not run (its script is not in the night's sync-perf copy, and no CLI query path moved).
- semver-checks against 0.19.1: every break has a BREAKING line; two were added (`sql::search::HIT_THREAD` removed; `PimdirIndex` no longer `Send`, holding the mmap of index.hits, which N6 now extends to a search-only build).

## For the owner

- Tasks §6: Tests and Releases green on all 8 targets; the signed tag v0.20.0 (the workflow publishes); pimdir's tags storage-draft-17, sync-draft-15, search-draft-17; each app then swaps its patch for `io-pimdir = "0.20"`.
- neverest's tests/queue.rs `a_malformed_row_fails_the_drain_coded` pinned G1: a malformed row used to fail every drain (exit 4) and is now parked (exit 3). The neverest lane updated its test (answers.md NV5).
- D10 is user-visible: a message queued unpushed and met by a renumbering is one message, where it was two.
- Deferred past 0.20.0, each with a workaround: HM1 (the owner's STORAGE §13 ruling), HM5, HM8, NV2, NV3, CD2, CD3, CD4, CL2, CR1, CR3, CR4, AN4.

## Not verified

- No live account and no local server used. The Android app's SQLite 3.18 plans of io-pimdir's statements stay unchecked (AN4). R8b and the drain are out of the oracle's reach; tests pin them.
