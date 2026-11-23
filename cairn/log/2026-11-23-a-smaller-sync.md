---
cairn: log
change: a-smaller-sync
landed: 2026-11-23
---

# A smaller sync

The sync night of pimdir cairn/changes/a-smaller-sync (proposal, design, moves, report there), run on 2026-11-23 from 131f9bf under one rule: every yield of a run but its loads, and every row it leaves, stays what 131f9bf gives. Each change below was gated by a differential oracle built that day (pimdir's .pimdir-night/sync-diff, not committed: the engine of 131f9bf and the tree side by side over the 200 vectors and generated multi-source histories, every yield but `WantsLoad` compared in lockstep, the rows and a battery of loads compared after each run; shown able to fail on three planted bugs first), by the whole suite, by neverest 99384ff's suite on a scratch port to the tree (727 tests), and by clippy, fmt and the no_std build.

- **Small helpers shared** (b4bcf37): one `is_mutable`, one split of a fetched body (`PimdirFetchedBody::stored`), rekey's local edit read through `staged_edit`, one `restaged` placement for sync and rekey. Oracle 20,000 scenarios, 0 differences.
- **One policy verdict** (0b862e2): `PimdirConflictPolicy::verdict` decides what a source's policy makes of a divergence for sync's `resolve_conflict` and upgrade's `settle`; each keeps its own effects, which differ (sync reports `Conflicted` and pushes, upgrade does neither). Oracle 20,000 plus 20,000 weighted to conflicts and upgrades, 0 differences.
- **A load naming no link reads no hub** (2f12b60): seven statements a folder fewer for a delta owing nothing. Two other store edits were measured and dropped within noise: cached `BEGIN`/`COMMIT` (0.5 ms of 75) and a statement cache keyed without the comment header (0.2 ms).
- **The collections a run may skip** (86f0421, spec sync.md "A settled collection may be left unsynced"): `PimdirSourceStore::settled_collections` over spec/queries/storage/owner/settled_collections.sql, tests/settled_collections.rs (9 tests, each way out shown to leave a collection out while another stays in), and the oracle syncing every settled end state anyway (16,814 of them) with no push, write or event. pimbench skips under `PIMBENCH_SKIP=1`. The read is 2 ms on the 1M store; a first form probing per collection took 3.6.
- **A whole collection read without copying it twice** (d9b79e6): the values streamed and filed one item lookup per link run, bindings, summaries and items streamed, the load's projection moving fields out of the hub (`project_moving`) where it cloned them. Every load's answer byte for byte the same on the 3,312 collections of the 517k store, and under the oracle's load battery.

The final tree against 131f9bf at the night's end: the 200 vectors and 100,000 generated scenarios (1,284,825 steps), 0 differences.

Measured on the 517k Enron store, idle machine, one hyperfine run each:

| | 131f9bf | d9b79e6 | notmuch new |
| --- | --- | --- | --- |
| no-op `pimbench sync`, every folder synced | 89.2 ms | 67.6 ms | 47.4 ms |
| the same, settled folders skipped | | 18.6 ms | |
| 300 new messages in one folder (median) | 234 ms | 212 ms | |
| an unchanged full round over 11,896 bindings (probe, in-process remote) | 408 ms | 308 to 341 ms | |

What the night found and did not change (each with its test in the change folder's candidates, reported in its report.md): rekey departs from pimdir SYNC §4 and §8 in nine cases, two of them silent losses (an owed flag push forgotten when a listing's flags are unknown; a conflict listed with no revision cleared); the queue drain keys a `Copy` or `Move` beside another source's item apart where `store.mutate` joins it. The test files that pin today's behaviour are in pimdir's .pimdir-night/candidates/k34.
