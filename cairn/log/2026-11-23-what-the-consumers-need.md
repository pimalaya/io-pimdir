---
cairn: log
change: what-the-consumers-need
landed: 2026-11-23
---

# What the consumers need, drops 1 and 2

The io-pimdir lane of the migration to storage-draft-17 (pimdir cairn/changes/the-consumers-follow-draft-17). Drop 1 (cc3021b, pushed) made CI green and the Android release build possible; drop 2 makes the rekey and the queue drain follow pimdir SYNC (sync-draft-15, edited in place at pimdir 0401aab) and answers the app lanes' accepted requests. Every rule below came with a check that failed on 3385039 and passes after; the decisions and timings are in cairn/changes/what-the-consumers-need/journal.md, the answers to each app in answers.md.

## Drop 1

- The Windows build compiles again (an untyped inode, src/client/search/hits.rs), and `mail`, `contact`, `calendar` and `indexer` build without `client` (`project_moving` under `client` or `test`): the CI matrix and its seven per-feature clippy runs pass, `nix build .#cross-x86_64-windows` yields pimdir.exe.
- `sql::search` builds under `indexer` alone, so an indexer binding its own SQLite takes no rusqlite (android AN1; tests/indexer_alone.rs). Capability: search.md.

## Drop 2, the rekey (rekey.md)

Each case is R0 to R8 of pimdir cairn/changes/a-smaller-sync (K3), each pinned by a pimdir vector and a test in tests/engine/rekey_cases.rs (the K3 harness, asserting SYNC):

- R4, a silent loss: an unknown listed flag set leaves the base's flags, so a flag change still owed is pushed (vector 201).
- R1, a silent loss: a conflict listed with no revision is carried as it is, its revision and diverging body kept (202).
- R5: a delivered create a member resolves to is landed as a page lands it, through the sync's own landing (`PimdirSync::land`, now shared): `Superseded`, its staged flags, the listed base with the staged body, `Created` (203).
- R6b, R6a (rule D10): a member joins a create the store offers its source (204), lands a pending create agreeing with it (205), and is minted apart and reported `Disagreed` when it disagrees (206). neverest's NV1 (no second `APPEND`) rests on 205.
- R2b, R2c: a conflicted tombstone keeps its conflict, listed at its base revision or its conflict's (207, 208).
- R3: a tombstone holding an edit met by a remote edit is a `Conflict` holding the listed body (209); the rekey reports `Overruled` from its write.
- R0: a repeated handle keeps its first entry (210).
- R8: an arrival's and a pull's listed body is stored at `Full` (211, 212).
- R2a (213) and R7 (D1, two tests: no vector can state a base-less binding) are pins, passing before and after.

## Drop 2, the drain and the reader (store.md, mutate.md)

- K4: the drain reads a `copy` or `move` target as a `Links` load does (`write::retain_links`), so it joins another source's item where it minted a `dup:` key (tests/drain_copy.rs); every mutate vector a queue action expresses drains to its rows (tests/vectors_sync.rs, 17 cases, none disagreeing).
- CD1 (G1; himalaya HM2, cardamum CD1, calendula CL1): a pending row whose payload does not decode is listed as `PimdirAction::Unknown`, kind and payload verbatim, the signature kept (D12); `pimdir queue list` prints its error. The generated reader twin runs again.
- AN3: `summary::DERIVATION` is public without `client`.
- CL3 and CR2 as documentation: a domain left out reads as a bodiless item; a reader keeps the file it opened.

## Checks

- The CI matrix on the tree (see journal.md for the counts at commit).
- The differential oracle (copy under ~/code/pimalaya/.lanes/io-pimdir/sync-diff, with a witness logger and `--expect`): drop 2 against drop 1 over 20,000 rekey-weighted scenarios, 2,578 differences, every one in a run where a fixed case's `trace!` line fired, 0 unexpected; 131f9bf against the tree over the 213 vectors, 12 differences, all expected. The 100,000-scenario run is in journal.md.
