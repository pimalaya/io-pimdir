# Journal

One line per decision or parked step, timed (2026-11-23, CET).

- 16:47 Lane started on io-pimdir 3385039. The owner took D1 to D13 and said yes to 0.20.0 in drop 3 (task 5.3), no tag.
- 16:50 Order: 0.1 and 0.2, then drop 1 (tasks §1) before the baseline 0.3 and the oracle 0.4, because the app lanes wait on drop 1 (the lead's instruction); 0.3 and 0.4 run once drop 1 is handed back.
- 16:52 C0: answers.md confirmed against the six files; new lines HM5, CD3, CD4, CL1 to CL3, CR1 to CR3.
- 17:01 Drop 1 gated: CI matrix on the tree (fmt, clippy all, the 7 feature sets, suite 1,180 passed 0 failed 8 ignored, `cli` 1,180, `--lib` 436), `--features indexer --test indexer_alone` 1 passed, `cargo tree --no-default-features --features indexer -e normal` names no rusqlite. sql::search's unit tests stay under `search` (they open rusqlite), so `vendored,indexer --all-targets` builds.
- 17:01 1.1: `nix build .#cross-x86_64-windows` builds the mingw gcc 15.3 from source (no substitute), still running at hand-back; left to finish, else the Releases run on the pushed drop decides (task 1.1's fallback).
- 17:01 Oracle copy at $L/sync-diff: ROOT and its tmp moved under $L (no /tmp), gate.sh and switch.sh pinned to CPUs 13,14,29,30 with `-j 4` and `--threads 4`, switch.sh cleans both sides, rebase.sh added (old/REVISION.lane names the base), the witness logger (src/witness.rs, cases.txt mapping a case to its fix's trace! line) and `--expect`. A difference is expected when a listed case fired in the run where it shows (the scenario, for the end battery).
