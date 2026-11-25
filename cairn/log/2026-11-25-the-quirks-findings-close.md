---
cairn: log
change: the-quirks-findings-close
landed: 2026-11-25
---

# The quirks findings close

The findings of cairn/log/2026-11-25-the-campaign-tools.md "Found on the way", reproduced from campaign-1's quirks seeds 1201 to 1208 (128 jobs: 19 replay and 8 crash jobs failing, 394 differences) and sorted by door; pimdir's entry of the same name holds the reasoning from the text. No pimdir text moves; pimdir vectors 234 to 236 are new, each failing on c8eb117 and passing here.

## Fixed in io-pimdir

- src/summary.rs `without_body`: a listing, a `Meta` fetch or a rekey keeps the whole summary of a message whose body the row holds, a `Message-ID` its server rewrote included; it kept the size and the mark alone before (finding 1; vector 236, cairn/spec/summaries.md). The shrinker drove `quirks:1204:replay:rewrites:4` to the same one-item case.
- src/sync.rs: a round page or a delta head landing a checkpoint sets it on the run's round plan, so a park drawing the round anew lands it again; a round opened and parked in one run left `round_checkpoint` `NULL` (finding 2; vector 234, cairn/spec/sync.md). Seen as the crash suite's `extremes` step 6: 931 members in pages of 61, uncut `NULL`, cut after any page `1036`.
- src/rekey.rs: every handle the rekey re-writes is dropped `Rekeyed`, one carried onto itself too, so the batch is a rebuild and the generation moves (finding 5; vector 235, cairn/spec/rekey.md). The unit test `a_reused_handle_is_not_dropped_by_the_batch_that_writes_it` asserted the old skip ("the batch decides by apply order", which STORAGE §14's ordered batch settles, vector 95 dropping and upserting one handle already) and is now `a_handle_carried_onto_itself_is_dropped_rekeyed_before_its_upsert`. The human `NOTE` above the drops in `PimdirRekey` still reads "every old handle but one carried onto itself is dropped": left for the owner to reword, the comment hook refusing an AI edit of it.

## Fixed in the generator

- spec/generator/src/engine.rs `refresh`: a listed message holding no body takes the listing's summary and addresses, any message the listing's known sort key, one holding its body keeping its summary (finding 4, SYNC §6, §9).
- `rekey`: the generation is bumped only when a binding was carried, its handle dropped `Rekeyed` (the shrinker's case of finding 5: an empty collection renumbered bumped it).
- src/queue.rs `apply_copy`: a create minted beside a live member under the bare hint holding the same body releases it from that member (`release_shared`, STORAGE §14), unless the member needs it (`Model::needs_body`, shared with `release_before`, STORAGE §11.4) or a member minted from the hint held it before (finding 6, count profile seed 1: club/Archive seq 1145 and home/Archive seq 758). Test: `a_move_minted_beside_its_body_in_the_target_releases_it_from_the_bare_member`, failing with the release left out.

## Harness

- tests/generated/crash.rs `cut_run`: the keeper holds a read transaction from before the run, so SQLite's automatic checkpoint backfills nothing and a first round of about 1,000 members is cut at every commit (finding 7; `extremes` step 3 reported "the run checkpointed the log" before).
- tests/generated/replay.rs and search.rs accept the `sender` references an owner MAY record (G21), through `scale::senders` and `scale::sender` as the scale suite does (finding 7; `renumbered` step 5 at seed 1207).
- tests/generated/campaign.rs: a replayed difference stands under a known finding by its suite, history and step label; one constant, `ETAG_KEPT_OVER_A_SERVER_EDIT`, for pimdir's Q1 (a DAV listing carrying another body at the base's ETag, `revisions` step 2). The campaign-tools entry read io-pimdir as adopting the carried body: it keeps the old body and writes the carried body's summary and sort key over it (seed 1201, seq 7: `object_hash` the old card's, `fn` "Edited Under One Etag"), a row disagreeing with itself whichever way the owner rules; left as it stands until the ruling.
- The working tree's tests/generated/search.rs and campaign.rs followed G28's removal of `query::Readings`, as main did; tests/generated/shrink.rs names `search::differences_of(model, false)` in the test it writes.

## Gates

- `cargo fmt --all --check`, `cargo clippy --all-features --all-targets -- -D warnings`: clean.
- `cargo test --all-features` and `--no-default-features --features cli`: 1,225 passed, 0 failed; `--no-default-features --lib`: 439 passed. The generator: fmt and clippy clean, 99 passed.
- The quirks profile over campaign-1's 160 quirks seeds (2,560 jobs, release): no failure, Q1's differences filed under `ETAG_KEPT_OVER_A_SERVER_EDIT`.
