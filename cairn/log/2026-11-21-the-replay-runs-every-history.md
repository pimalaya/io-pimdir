---
cairn: log
change: the-crate-follows-draft-15
landed: 2026-11-21
---

# The replay runs every history

tests/generated/replay.rs now replays the generator's four newest histories through io-pimdir, `rekeys`, `crashes`, `heads` and `bodies`, beside the eight it ran (pimdir cairn/changes/the-freeze-plan/tasks.md, "Left on 2026-11-20"). tests/generated/crash.rs cuts their runs after every commit as it cuts the others'.

## The harness

- The scripted connector answers a run's head (`PimdirHeads::DELTA` when the run lists one), rejects the cursor a restarted round resumed from, lists a band where the run's round is a band (`scope_bound` false), fails a round the model cut between two pages with a page before the last handing no cursor, and fails the push after which the model's run crashed, its outcome never written. A run the model says failed must fail, its events left unread (the write it failed in reports nothing, SYNC §5).
- `rekey` runs `PimdirStore::rekey` with the run's `accept_absences`; `mutate` runs the overruling `Edit` (SYNC §7) with the body the item holds after it, or before it where the model refuses it; an upgrade raises the run's `handles` rather than its fetched ones.
- Harness bugs fixed: `declared` gave every early step a collection as the last step held it, so a rekeyed collection's generation read 4 at its first round; the convergence check counted the `sync_version` stamp an engine's first run writes on a store the generator wrote as a moved row; a crash rerun answered the cursor a resumed round started from with no page, and rejected a restarted round's own cursor again.
- The four trial divergences named in the freeze plan's tasks: bodies steps 4, 9 and 14 were G14's tag over what is G25 below; step 26 replays clean once step 25 is named; step 33 was the `sync_version` stamp.

## Fixed in io-pimdir (each with a test failing before it)

- src/rekey.rs `carry`: a local edit carried in conflict onto its new handle took the summary and sort key of the server's body the fetch read, the item keeping the edit (`rekeys` step 9, the card at seq 26 read "Their Version" over the body "Our Version"). SYNC §8 carries the placement's summary with its body; tests/engine/summary_kept.rs `a_rekeyed_edit_in_conflict_keeps_its_summary`.
- src/upgrade.rs `write_fetched`: a `Full` fetch under a local edit gave the item the base's summary and sort key, the edit staying its body (`rekeys` step 11, "their version" over the body "Edited Unread"). SYNC §6; tests/engine/upgrade_summary.rs `an_upgrade_reading_the_base_under_an_edit_keeps_its_summary`.
- src/hub.rs `absorb`: a bodiless upsert over a body the item keeps took the upsert's summary when a second upsert of the same placement in the batch (content, then flags) met no divergence left to keep it (`bodies` step 31, "edited at b" over the body "Edited At A", recorded as the item's unread divergence). SYNC §5, §9; the hub's `summary_kept_tests::a_bodiless_upsert_over_a_kept_body_keeps_its_summary`.

## Findings recorded (tests/generated/replay.rs, the `#[ignore]` twin)

- G23 (spec, reads two ways): the flags a rekey's listing states. SYNC §8 "carries each placement's [...] flags, base" is read by the generator as kept as they stood, by io-pimdir as merged with the listed flags, the listed ones the base. Evidence: `rekeys` step 3, two copies of one `Message-ID` matched in handle order, the oracle `draft.6` flags `""` where io-pimdir holds `\Seen` (the listing's), the model's base claiming `""` for a member listed `\Seen` past the rekey's checkpoint. The model read with the merged flags agrees with io-pimdir on every statement. For the owner: state whether a rekey runs §5's flag axis.
- G24 (the generator's): a rekey's pull keeps the old summary and sort key (`rekeys` step 9, seq 27: the oracle "Hugo Leroy", io-pimdir "Their Version"), against SYNC §8 "carry it as a pull (§5)" and §5's pull making the member's meta the summary and sort key.
- G25 (the generator's): an upgrade reports `Conflicted` (`bodies` steps 4, 9, 14), against SYNC §5 "an upgrade and a mutation report none, save `Overruled`". The store agrees on every statement.
- G26 (the generator's, minor): a bodiless message reviving a retained row sets `shared_object` (`bodies` step 21), against STORAGE §10's "moved by an absorbed upsert carrying a body [...], never by a pull carrying none"; io-pimdir leaves it `NULL`.
- G14 turned over: SYNC §5 (sync-draft-15) now says a binding owing a body another source folded is pulled and folded whatever its policy, the divergence the collection's. io-pimdir follows it (`folded`); the generator records the binding's conflict (`bodies` steps 25 and 31). G14's text now names it the generator's.

G12 tags `bodies` step 18 (a revive's kept body keeping the winner's sort key), the finding it was.

Capabilities moved: rekey, upgrade, hub.
