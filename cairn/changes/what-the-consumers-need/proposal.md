---
cairn: change
id: what-the-consumers-need
status: active
created: 2026-11-23
---

# What the consumers need

Read pimdir cairn/changes/the-consumers-follow-draft-17/frame.md first. This lane alone edits io-pimdir and pimdir while six lanes move their apps onto io-pimdir 7f16b27. Its scratch is ~/code/pimalaya/.lanes/io-pimdir (`$L`); §n means inventory.md's sections.

## Why

- The rekey departs from SYNC in the K3 cases, two of them silent losses, and the drain in K4. All 23 tests that pin them still pass on 7f16b27 (§1).
- master's CI is red (§3). Releases fails the Windows build, and on a tag the Linux job publishes before Windows builds, so a tag now would publish and then fail. Tests fails four per-feature clippy runs.
- The apps leave their git pins only once 0.20.0 is released.

## What

1. **Drop 1, CI green**: src/client/search/hits.rs:128 and src/hub.rs:465 (§3), plus `sql::search` under `indexer`, which the Android app's release build needs.
2. **Drop 2, the engine follows SYNC.** Each fix lands with a pimdir vector that fails on today's engine, the manifest restated, the suite and the oracle (pimdir AGENTS.md §4). The verdicts and their lines are in §2:

| Case | Verdict | Fix |
| --- | --- | --- |
| R0 | bug, SYNC §4 keeps a duplicate's first entry | the first entry, whole |
| R1 (loss) | bug, §8 a conflict is carried as it is | revision and body kept when no revision is listed |
| R2b, R2c | bug, §3 and §5: a delete settles no conflict | tombstones carry the conflict |
| R3 | bug, §8 a `Conflict` at the fetched revision | the both-changed arm first; `Overruled` reported |
| R4 (loss) | bug, §8 an owed flag is pushed; §3 | an unknown listed set keeps the base |
| R5 | bug, §8 as a page lands it | `Superseded`, staged flags, listed base, `Created` |
| R6b | bug, §8 named as a page names it | offered copies take no key |
| R6a | rule: §8:546 counts a pending create's key taken, yet a page lands it (§6) and neverest shows the second `APPEND` | D10 |
| R7 | rule: "carried as it is" assumes a provisional handle | D1 |
| R8 | bug, §8 arrival as a page, edit as a pull | listed bodies stored |
| K2 | rule: §5 says mail carries no revision | D2 |
| K4 | bug, §7 and §10, STORAGE §14 | the drain projects as `load(Links)` |

R2a is as SYNC states; a vector pins it. sync-draft-15 is untagged, so SYNC.md is edited in place (AGENTS.md §2 bumps after a tag only). storage-draft-17 and search-draft-17 do not move.

3. **The oracle**, a copy of .pimdir-night/sync-diff in `$L`. For each fix, its old side is re-based on the lane's commit before that fix, so any difference is the fix's. Each fix branch logs one `trace!` line. Gate: every first difference falls in a run where that line fired. Zero fires means the generator misses the case and the vector stands alone. A final run compares 131f9bf with the tree. The oracle never drains, so for K4 it only shows 0 differences; a drain-against-mutate parity test proves K4.
4. **Requests.** The lane reads the six <app>/cairn/changes/follows-storage-draft-17/needs-io-pimdir.md at C0 to C3 and answers in answers.md: accepted (task, drop), declined, or deferred past 0.20.0. A request is in when a migration or a loss needs it first. A drop is marked in drops.md with what consumers feel, and archived to $L/drops/<n>/ for path-patched scratch builds. Once the owner re-signs and pushes it, the lane records the hash and the app lanes re-pin to it. There are three drops: CI, fixes, RC. The four files that existed at planning are folded into answers.md (§9).
5. **Open work** (§4): of the 80 folders, 71 are archived, 1 is in (the consumers' feature lines in the changelog), 8 are out. There are no issues.
6. **Release preparation** (tasks §5), with the owner's steps after (tasks §6).
7. **Performance**: 7f16b27 against the RC on a copy of the 517k store, CPU 15, under the measure lock. Measures: the no-op sync both ways, 300 new messages, the 17 queries, the rekey scale test. A measure fails at more than 3% slower after one re-run.

## Out

- W1, W2, W3 (moves.md): each changes the wire (a load count, a selection, the batches the Java store re-runs).
- C1 and C4 to C7: rule changes nobody asked for.
- SYNC-rewrite.md: its own review, not mixed with fixes.
- The 8 folders and the generated findings of §4 and §6.
- Deferred requests NV2, NV3, CD2, AN4 and HM1 (answers.md): none blocks a migration.

## Decisions (defaults taken)

- D1 R7: keep today's outcome (carried with a base of the listed flags, or re-staged when unmatched). SYNC §8 states it; vectors 213 and 214 pin it.
- D2 K2: unchanged in 0.20.0. neverest states no revision for mail (§2).
- D3 Clauses added to SYNC: §8 for an unknown listed set, §5 for a rekey's `Created` and `Overruled`.
- D4 The 71 folders go to archive/, references re-pointed, logs as written.
- D5 Apps pin pushed hashes only.
- D6 The witness is a `trace!` line in each fix branch. Fallback: precondition predicates in the copy.
- D7 517k only: the 1M stores are gone, and no fix touches the query, index or write paths.
- D8 Not in 0.20.0: `PimdirFetchedItem::server_object_id` (no connector can state one yet) and G13 (neverest mirrors `hash:` items).
- D9 tests/ stays in the package (§5).
- D10 R6a: a rekey member lands this source's pending create as a page does, and SYNC §8:546 changes. A user sees one message where a second `APPEND` made two.
- D11 `non_exhaustive` waits for the 1.0 API review.
- D12 G1 is fixed only in a shape keeping `pending_actions`' signature.
- D13 Whether a system flag's case is folded into `marks` (himalaya HM1) stays the owner's STORAGE §13 ruling, not made in 0.20.0.

## Risks

- Re-signing rewrites hashes, so an unsigned pin breaks (D5).
- The oracle may reach none of R0, R1 or K4; their checks are the vector and test alone.
- A tag publishes before Windows builds: Releases must first be green on the master push.
- The fixes move what neverest's rekey writes: neverest's suite on a scratch copy gates drop 2.

## True before the owner tags

Every app lane is green on drop 3's pushed hash, the cross-app store check included (frame §8 J). io-pimdir passes its suite and CI matrix with tests/pending_vectors.txt empty, and Tests and Releases are green on the push. pimdir passes `nix flake check` and checks/implementation.sh at the RC. The oracle chain shows no unwitnessed difference, no measure fails, and tasks §5 is done.
