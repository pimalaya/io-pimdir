# Inventory, 2026-11-23

io-pimdir 7f16b27 and pimdir e40b258, as they stood on 2026-11-23. "Measured" means run today; logs are under ~/code/pimalaya/.pimdir-migration/io-pimdir/. Everything else is read from code or docs and is unverified.

## §1 The suite with K3 and K4 (measured)

- The snapshot (`git archive HEAD`, with candidates/k3_rekey.rs and k4_drain.rs added to tests/, pimdir linked beside it) ran `cargo test --all-features --no-fail-fast -j 4` on CPUs 13,14,29,30 from 15:22:58 to 15:27:05. Result: 1,203 passed, 0 failed, 8 ignored over 91 binaries (suite-7f16b27.log). That is the night's 1,180 plus the 19 K3 and 4 K4 tests.
- All 19 k3_* tests and the 4 k4_* tests pass, so every behaviour they pin is still there. spec_drift passed 7 of 7, comparing against pimdir.

## §2 Each case (read from code at 7f16b27, reproduced by its K3 or K4 test)

| Case | Code | SYNC (pimdir e40b258) |
| --- | --- | --- |
| R0 | src/rekey.rs:172 `dedup_by` keeps the first entry; :640-648 `resolved.insert` keeps the last | SYNC.md:142 "keep a duplicate's first entry". The last entry's link id leaves the carried item dropped `Deleted`, which §9 propagates to other sources as a `Remove` |
| R1 | :430-435 nothing is observed when no revision is listed; :491-499 the conflict revision becomes that nothing, and the body goes with it | :552 "carried as it is ... while the fetched revision is the one recorded"; :344 a newer revision alone moves it |
| R2b, R2c | :454-470 the carried placement starts with no conflict; the tombstone arms at :503-515 never restore it | :98 "A delete settles no conflict"; :369 a conflicted tombstone derives no `Remove` |
| R3 | the arm at :503 (tombstone, remote edited) matches before :516 (remote edited and local edit) | :550 "or as a `Conflict` at the fetched revision when it also holds a local edit"; :369 |
| R4 | :466-467 the base takes the listed set, unknown included | :530 "A flag change still owed is pushed by the next sync"; :104 an unknown set holds no opinion; :562 |
| R5 | :207-211 carried, dropped `Rekeyed` at :293-295; flags merged at :428; no `Created` | :538 "carried onto it as a page lands it (§6)"; :437-439; :413 |
| R6b | :158-160 every `Created` placement is staged and :179-182 claimed; :598 drops `loaded.offered` | :536 "named as a page names one"; :546 covers pending creates; :398 "A create the source binds not, one the store offers it"; :425 joined |
| R7 | :246-263 an unmatched one is re-staged; :466-470 a matched one is given a base | :538 "A binding with no base is outside the rekey and is carried as it is" |
| R8 | rekey.rs writes no `StoreObject`; :504-505 and :521-522 pulls drop the body; :556-575 an arrival lands at `Meta` | :536, :550 and §5 :338 ("the member's body, when it carries one, becomes the body") |
| K2 | the engine pulls; the candidate records a conflict (a-smaller-sync report.md) | :342 "Mail reports no revision and never reaches this axis". neverest src/sync/remote.rs:12-13: an immutable kind leaves the revision `None` |
| K4 | src/client/producer.rs:2209-2210 projects the target with offered copies; src/client/write.rs:196-204 `Links` keeps items the source binds | :482 actions map onto mutations; :620; :632; STORAGE.md:422 `Links` "never offering a create for an item the source lacks" |

R2a and R6a: SYNC.md :562 (a tombstone's flags ride along) and :546 (pending creates' keys count as taken). The candidates' tests pin both. R6a contradicts :437 (§6: a page lands a pending create rather than mint) and :328 (pushing first makes a second copy), so it becomes rule D10 (§9).

## §3 CI (measured with gh and locally)

- Releases has failed on every master push since run 45022513048 (2026-11-22 12:29); the last success was 45020459082. Job `x86_64-windows`: `error[E0689]: can't call method rotate_left on ambiguous numeric type {integer}` at src/client/search/hits.rs:128 (run 45029745767, on 00727ba) and :129 (45190373522, on 7f16b27). `git log -S 'let ino = 0;'` gives 85503f4.
- pimalaya/nix .github/workflows/release.yml: on a tag, the x86_64-linux job checks the tag against `cargo metadata`, builds and runs `cargo publish --no-verify`; every other target `needs` it and runs after.
- Tests run 45190372690, step "Each search feature builds alone". Replayed locally (features-7f16b27.log): `vendored,mail`, `contact`, `calendar` and `indexer` fail with `method project_moving is never used` (src/hub.rs:465); `search`, `search,mail` and `search,indexer` pass; `cargo fmt --check` passes. On 00727ba the same step failed on `posting: bool` never read.

## §4 The 80 change folders (headers and open lines, 15:30)

The counts by header: 69 landed, 6 active, 4 draft, 1 abandoned.

Archive, landed with no open line (53): acknowledged-intents-leave-receipts, a-delivered-create-waits-for-its-arrival, a-landed-create-holding-another-body-conflicts, a-listing-proves-what-it-drops, an-ambiguous-intent-awaits-confirmation, a-narrowed-round-proves-only-what-its-enumeration-lacks, an-automatic-settle-keeps-what-it-discards, an-expansion-bounded-by-its-instances, an-item-keeps-local-state, an-unreadable-body-is-retried-then-parked, an-unread-arrival-landing-a-create-conflicts, a-page-without-a-cursor-is-refused, a-producer-proves-it-can-write, a-query-only-index, a-query-states-its-zone, a-queued-edit-names-what-it-changes, a-search-answers-at-scale, a-streamed-body-is-verified-at-its-length, collection-create-occurrences-receipts, engine-conformance, errors-carry-a-stable-code, every-read-takes-an-older-store-as-reconciled, gtd-hardening, linked-and-streamed-body-columns, machine-output-contract, one-recognition-of-a-minted-key, one-refusal-for-another-version, only-the-sync-engine-migrates, release-hygiene, revised-standard-alignment, round-head, rows-and-questions-draft-06, scoped-mail-sync, stand-ins-as-the-body-enters, store-audit-2026-10-30, store-conformance, sync-audit-2026-10-30, tasks-by-categories, the-agreed-body-stays-pinned, the-crate-follows-draft-14, the-crate-follows-the-freeze-gaps, the-default-enqueue-strips, the-engine-follows-sync-draft-05, the-index-reads-the-canonical-columns, the-json-follows-pimalaya-cli, the-logger-starts-inside-the-result, the-producer-stages-its-own-body, the-store-follows-storage-draft-05, the-store-version-contract, the-text-output-is-plain, what-pimdir-stated-for-the-store, what-the-app-ports-found, wide-lists-print-as-records.

Archive, abandoned (1): a-page-lands-in-few-statements.

Archive, its open lines someone else's, done or out (17):

| Folder | Open lines | Verdict |
| --- | --- | --- |
| a-conflict-never-hides-a-server-edit | neverest picks it up | neverest's, long done |
| a-delta-loads-what-it-names | last page keeps `All`; a bench | W1, out |
| an-in-memory-card-matcher | cardamum calls it | cardamum's |
| an-io-free-indexer | a review with the owner | the owner's, no release gate |
| a-page-sorts-its-own-hits | threads' statement per hit | done by fcb6012 (`HIT_THREADS`, `THREADS_HELD`) |
| a-pending-create-is-withdrawn-whatever-the-rights | calendula's capabilities | calendula's |
| a-purge-of-chosen-items | neverest's account-scoped purge | deferred unless neverest's needs file asks |
| a-refused-push-shows-on-its-receipt | REVISION; connectors answer `Refused` | done; consumers' |
| a-runner-of-its-own-counts-its-verb | the Android runner | android's |
| as-fast-as-notmuch | 8 perf lines | done by the two nights, or out |
| charset-tables-for-legacy-cjk | neverest's feature line | neverest's |
| one-no-replace-move | Android `PimdirBlobMove` | android's |
| the-crate-follows-storage-draft-08 | 6 consumer and pimdir lines, `match_objects_ranked` | superseded by this migration; out |
| the-json-output-is-the-integration-point | 5 contract limits before 1.0 | out, the freeze plan's API review |
| the-merge-verdict-is-public | Android `reconcile` | android's |
| the-store-rules-the-clis-restate | the CLIs drop their copies | the CLIs' |
| the-crate-follows-draft-15 (active) | D2, the query language | superseded by draft-17: landed |

In (1): the-query-split-by-domain (draft): "CHANGELOG, 0.20.0, consumers' feature lines" goes to task 5.2.

Out (8):

| Folder | Why out of 0.20.0 |
| --- | --- |
| an-owner-write-checks-its-fence | a STORAGE format change: it would refuse every store mid-migration |
| a-push-reads-back-the-server-bytes | a SYNC §4 wire change, and its connectors |
| a-same-id-message-keeps-its-body | needs `EMAILID`, which io-imap 0.12 cannot request; owner rulings; D8 |
| deep-mime-regression | a test for a fix already landed; no consumer feels it |
| derivation-before-the-write | lock-hold perf; write path closed 2026-11-13 |
| raw-payloads-print-their-length | trace-log hygiene; the non-breaking form can ship in 0.20.x |
| search-feature | a milestone tracker; its lines are pimdir's and the consumers' |
| a-decryption-break | waits for decryption to be planned (owner, 2026-11-21) |

`gh issue list --state all -R pimalaya/io-pimdir`: none, ever. `gh pr list`: none open.

## §5 Release probes (measured, release-probe.log)

- `cargo deny check`: advisories, bans, licenses and sources ok, with two warnings: MPL-2.0 is allowed but not met, and the `pimalaya` org allowance is not met since no git sources remain.
- `cargo package --list --allow-dirty`: 506 files, about 5.9 MB uncompressed, tests/ 2.6 MB of it. spec/queries (245 files) and spec/migrations are shipped; spec/generator, spec/vectors and spec/payloads are excluded.
- Toolchain: the devshell's rustc is 1.99.1; Cargo.toml declares `rust-version = "1.89"` and uses let-chains (src/rekey.rs:478), which need 1.88.

## §6 Other ledgers (read)

- pimdir audit-2026-11-12 open lines with an io-pimdir part: A5 and A6. Both ask to make public `client::schema::DERIVATION` (3, which the Android app copies by hand) and `summary::same_message`. Out: the value does not move in 0.20.0. C5 is the CLIs'; C3 is pimalaya-cli's.
- pimdir the-freeze-plan §6 (the API review, `non_exhaustive`, semver-checks at 1.0): before 1.0, out. Line 69, an owner write registering a source, is likely a neverest request (neverest src/sync/storage.rs:143-182 writes `sources` itself).
- tests/generated known findings, as ignored twins: G1 (one malformed pending row fails a listing), G12, G13 (`hash:` offered; D8), G15 to G17, G20 and G21. All out.

## §7 The oracle's reach (read from .pimdir-night/sync-diff)

- Unknown flags in 5% of draws (src/scen.rs:131-135), mutable sources in 35%, bodies carried in 40% (src/scen.rs:200-202), and a rekey after 70% of renumberings. It drives two or three sources.
- Not reached (README "Blind spots"): duplicate handles in a page, a mutable listing without revisions, the queue drain. The re-based old side is the old/ directory with `name = "io-pimdir-old"`, nothing else changed.
- 165 scenarios a second on 16 workers, so about 40 on 4: 20,000 take about 8 minutes, 100,000 about 40.

## §8 Performance baselines (read)

- The sync night at 517k (a-smaller-sync report.md): no-op 67.6 ms with every folder synced, 18.6 ms skipping settled ones, 300 new 212 ms.
- The query night (a-smaller-pimdir report.md): 51 measures, but at 1M, whose stores were deleted (frame §5). The 517k store, its corpus and state remain under .pimdir-night/sync-perf.

## §9 App requests (read at about 15:45 on 2026-11-23)

- neverest/cairn/changes/follows-storage-draft-17/needs-io-pimdir.md (13 lines): NV1 to NV4.
- cardamum/…/needs-io-pimdir.md (22 lines): CD1 (G1), CD2 (`non_exhaustive`).
- android/…/needs-io-pimdir.md (35 lines): AN1, which they measured: `indexer` alone fails on `sql::search` with E0432, and `indexer,search` fails to link `-lsqlite3`. Also AN2, AN3 and AN4. R0 to R8 and K4 do not reach the app: it runs no rekey and drains no copy (read from code).
- himalaya/…/needs-io-pimdir.md (read at about 15:50): HM1 (system flag case in `marks`; spec/migrations/storage/0001_init.sql:295-298 stores marks by exact bytes), HM2 (G1), HM3 (R4, R1, K4) and HM4.
- calendula and carillon: no file yet.
- Each is answered in answers.md. R6a joins the rules because NV1's test, a queued unpushed `add` met by a renumbered member, expects no second `APPEND`, while SYNC.md:546 (keys taken) mints the member apart and the create is pushed again.
