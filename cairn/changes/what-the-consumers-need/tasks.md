---
cairn: tasks
change: what-the-consumers-need
---

# Tasks

Conventions: `I` = /home/soywod/code/pimalaya/io-pimdir, `P` = /home/soywod/code/pimalaya/pimdir, `L` = ~/code/pimalaya/.lanes/io-pimdir. Every cargo runs as `CARGO_TARGET_DIR=$L/target CARGO_PROFILE_DEV_DEBUG=0 taskset -c 13,14,29,30 nix develop $I --command cargo … -j 4`. The suite is `cargo test --all-features --no-fail-fast`. The CI matrix is the suite, `--no-default-features --features cli`, `--no-default-features --lib`, fmt, `clippy --all-features --all-targets -D warnings` and the per-feature loop of .github/workflows/tests.yml (inventory §3). Commits follow frame §4. Each decision is timed in journal.md. A gate failing twice parks its step (frame §7).

## 0. Start

- [ ] 0.1 Create journal.md and drops.md here. Gate: they exist.
- [ ] 0.2 C0: read the six <app>/cairn/changes/follows-storage-draft-17/needs-io-pimdir.md, confirm answers.md's lines and add the new ones. Gate: every request has a line.
- [ ] 0.3 Baseline: run the suite on a `git archive HEAD` snapshot at $L/src, with P/cairn/changes/a-smaller-sync/candidates/k3_rekey.rs and k4_drain.rs in tests/ and $L/pimdir a symlink to P. Gate: 1,203 passed, 0 failed, 8 ignored (inventory §1).
- [ ] 0.4 Oracle copy: `cp -a` .pimdir-night/sync-diff, without target/, to $L/sync-diff. Set D, `-j 4`, `--threads 4` and taskset in gate.sh and switch.sh. Add rebase.sh `<commit>` (git archive into old/, `name = "io-pimdir-old"`). Add a logger recording `trace!` lines per thread and `--expect <cases>`, which accepts a first difference only in a run where a listed case's line fired and reports the differing, expected, unexpected and fired counts. Gate: 131f9bf against the tree, 200 vectors and 100,000 scenarios, 0 differences.

## 1. Drop 1: CI green and Android's build

- [ ] 1.1 I/src/client/search/hits.rs:128 becomes `let ino: u64 = 0;`. Then `nix build .#cross-x86_64-windows --no-write-lock-file --max-jobs 1 --cores 4`. Gate: result/bin/pimdir.exe. If no cross toolchain is reachable, journal it and let the Releases run decide.
- [ ] 1.2 I/src/hub.rs:465 `project_moving` under `#[cfg(any(feature = "client", test))]`; its callers are src/client/write.rs:182 and the hub test at :1253. Gate: the CI matrix, all 7 feature sets.
- [ ] 1.3 AN1: I/src/sql.rs:18 `#[cfg(any(feature = "search", feature = "indexer"))]`, with build.rs baking the index statements for `indexer` too. Add tests/indexer_alone.rs, `#![cfg(all(feature = "indexer", not(feature = "client")))]`, naming `sql::search::FORMAT_VERSION` and `all()`. Gate: `cargo test --no-default-features --features indexer --test indexer_alone` passes, and `cargo tree --no-default-features --features indexer -e normal` names no rusqlite.
- [ ] 1.4 CHANGELOG Unreleased, Fixed and Added, then commit. Drop 1 goes in drops.md, with `git archive` to $L/drops/1. Then C1 (as 0.2).

## 2. Drop 2: the engine follows SYNC, and the accepted requests

The case loop, for 2.1 to 2.9:
1. Write P/vectors/sync/<n>-<slug>.json and a sentence in P/vectors/README.md; edit P/SYNC.md only where the row says.
2. `nix-shell -p python3 --run 'python3 -I checks/vectors.py . --manifest > vectors/manifest.json'`, then `nix flake check path:$PWD`, and commit P.
3. Re-vendor I/spec/vectors and set spec/REVISION to P's commit. Run `cargo test --all-features --test vectors_sync`. Gate: the new case fails, its failure journalled.
4. Write the fix, and turn the case's K3 test into tests/engine/rekey_cases.rs asserting SYNC's outcome. Gate: the suite and clippy.
5. `./rebase.sh <commit before>`, then `./gate.sh <case> 20000 --weights rekey --expect <case>`. Gate: 0 unexpected, counts journalled.
6. Commit I. Set P/checks/io-pimdir.rev to it and commit P.

- [ ] 2.1 R4 (201): a binding owes `\Flagged`; the listing's flags are null. Expect the base `\Seen` kept and no push. src/rekey.rs:466-467. SYNC §8 gains "an unknown listed set leaves the base's".
- [ ] 2.2 R1 (202): a conflict at "1" with its body, listed with no revision. Expect both kept. :489-500.
- [ ] 2.3 R2b (203), R2c (204): a conflicted tombstone listed at its base revision, then at the conflict's. Expect the conflict carried, no `Remove`, no revive. :503-515.
- [ ] 2.4 R8 (206 arrival, 207 pull): listed bodies, inline and persisted. Expect `StoreObject`, level 2, the pull's base body. :454-575.
- [ ] 2.5 R3 (205): a tombstone holding an edit, remote edited. Expect a conflict at "1" with the listed body, the edit kept, nothing retained. :503 before :516. The report carries `Overruled` for `PimdirWritten::overruled`, and SYNC §5 lists it for a rekey.
- [ ] 2.6 R0 (208): a repeated handle. Expect the first entry whole and no `Deleted`. :639-649.
- [ ] 2.7 R5 (209): a delivered create matched. Expect a `Superseded` drop, staged flags, the listed base with the staged body, `Created`. :202-211, :293-295, :475-483. SYNC §5 lists `Created` for a rekey.
- [ ] 2.8 R6b (210): an offered copy's hint stated by a member. Expect a join, one item. :158-182, and keep `loaded.offered` (:598).
- [ ] 2.9 R6a (212, D10): a pending create's hint stated by a member. Expect it landed as §6 lands it, or minted apart and `Disagreed` when the summaries disagree. SYNC §8:546 is edited. This is what neverest's NV1 asks.
- [ ] 2.10 K4: tests/drain_copy.rs from candidates/k4_drain.rs, asserting that the drained Copy and Move leave `store.mutate`'s rows (fails before). One `pub(crate)` `Links` projection (src/client/write.rs:196-204) serves src/client/producer.rs:2209-2210. Then tests/drain_agrees.rs: each `mutate` vector a queue action expresses, its item bound by one source, drained to the vector's rows. A further disagreement goes in report.md (a loss first) and is listed as known in the test with its reason. Gate: suite, and 0 oracle differences.
- [ ] 2.11 CD1 (D12): the ignored G1 twin in tests/generated/reader.rs is the check (fails before). `pending_actions` (src/client/producer.rs:717, reader.rs:1237) lists an undecodable row as an opaque action naming its error, with the signature kept. Gate: G1 leaves the known list. If the signature must break, defer CD1 and say so in answers.md.
- [ ] 2.12 AN3: `pub const DERIVATION` outside `client`, which src/client/schema.rs:26 re-uses. CHANGELOG Added.
- [ ] 2.13 Pins passing before and after: R2a (211), R7 matched (213) and unmatched (214). SYNC §8 states D1.
- [ ] 2.14 Close the drop:
  - final oracle, 131f9bf against the tree, 100,000 scenarios, `--expect` every case: 0 unexpected;
  - neverest's lane head (or .pimdir-night/neverest if it does not build) as a git archive, path-patched to I, suite at drop 1 then at the tree, no servers and no live: no new failure;
  - delta.md folded into cairn/spec; CHANGELOG Fixed, BREAKING where rekey outcomes move;
  - logs I/cairn/log/<date>-what-the-consumers-need.md and P/cairn/log/<date>-the-rekey-and-the-drain-follow-sync.md, each rule with its vector;
  - K3 and K4 ticked in P/cairn/changes/a-smaller-sync/tasks.md; AN2 in P/cairn/changes/a-smaller-pimdir/consumers.md;
  - commit, drop 2 as 1.4, then C2.

## 3. Housekeeping

- [ ] 3.1 Move the 71 folders of inventory §4 to cairn/changes/archive/, setting the-crate-follows-draft-15 `landed`, and re-point references outside cairn/log. Gate: no `cairn/changes/<id>` left outside cairn/log and archive/.

## 4. Performance

- [ ] 4.1 Under `flock -s ~/code/pimalaya/.lanes/locks/store-517k`, copy .pimdir-night/sync-perf store/ and state/ to $L/perf (`cp -a`) and corpus/ (`cp -al`). Point prep-incr.sh at it.
- [ ] 4.2 Build pimbench (I/bench) and the CLI at 7f16b27 twice (for the A/A) and at the RC, three copies each on ext4.
- [ ] 4.3 Under `flock ~/code/pimalaya/.lanes/locks/measure`, `taskset -c 15`, hyperfine interleaved, after an A/A within 1%: the no-op sync (every folder, then `PIMBENCH_SKIP=1`), 300 new (prep-incr.sh), scripts/queries.sh at 517k, and `cargo test --release --test engine rekey_scale -- --ignored --nocapture`. Gate: no median more than 3% slower after one re-run. The table goes in report.md.

## 5. Drop 3: the release candidate

- [ ] 5.1 C3. Gate: answers.md complete.
- [ ] 5.2 CHANGELOG: Unreleased becomes `## [0.20.0] - <date>`, with an empty Unreleased above. Add the consumers' feature lines (the-query-split-by-domain proposal.md:34-39) and the start-over note (frame §2). Set that change landed and archived.
- [ ] 5.3 Only with the owner's yes to 0.20.0, asked when the lane is launched (never bump a version unasked): Cargo.toml `version = "0.20.0"`, Cargo.lock updated. Gate: `cargo metadata` reads 0.20.0. Without the yes, drop 3 keeps 0.19.1 and journal.md says so.
- [ ] 5.4 MSRV: `cargo check --all-features --all-targets` with fenix 1.89 (a scratch shell.nix). Gate: builds; if not, the first version that builds goes to the owner.
- [ ] 5.5 `cargo deny check`, `cargo package` (verify), and `cargo package --list` against inventory §5. Gate: both pass.
- [ ] 5.6 `nix-shell -p cargo-semver-checks --run 'cargo semver-checks --baseline-version 0.19.1 --all-features'`. Gate: every break has its BREAKING line.
- [ ] 5.7 The CI matrix, then drop 3 as 1.4, then report.md (frame §8 L).

## 6. The owner's, listed

- [ ] 6.1 Re-sign and push I and P (frame §4). Tests and Releases green on that push, all 8 targets.
- [ ] 6.2 Every app lane green on drop 3's hash (proposal, last section).
- [ ] 6.3 Signed tag v0.20.0; the workflow publishes.
- [ ] 6.4 P/checks/io-pimdir.rev set to v0.20.0's commit, and the tags storage-draft-17, sync-draft-15 and search-draft-17 at that P commit.
- [ ] 6.5 Each app swaps its patch for `io-pimdir = "0.20"`.
