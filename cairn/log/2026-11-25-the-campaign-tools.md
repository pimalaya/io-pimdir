---
cairn: log
change: the-campaign-tools
landed: 2026-11-25
---

# The campaign tools

The tools of pimdir's freeze-plan campaigns (pimdir cairn/changes/the-freeze-plan, section 2 "Shrinking" and "Profile: quirks", section 7 "The campaign's seeds and profiles fixed and recorded"), the campaigns themselves not run. spec/generator/README.md, "Profiles", "Shrinking" and "Campaigns", is their manual.

## What landed

- spec/generator/src/quirks.rs: the quirks profile, eight histories (`renumbered`, `listings`, `message-ids`, `revisions`, `narrowed`, `relocations`, `rewrites`, `extremes`) over remotes renumbering handles, listings unsorted, repeating a member with other flags and dropping members, a `Message-ID` reused across mailboxes or absent, an ETag that stays put over a server edit and over a pushed one, capped and narrowed rounds with and without `enumerated`, received dates skewed from the `Date`, relocations stating the handle in `to` (`quirks::land`, SYNC §4) or none, an appended message whose `Message-ID` the server rewrites (answered with a handle or none) and a received one gaining a header, an empty mailbox, one of 300 to 1,000 members losing more than half, one emptied whole. The seed draws sizes, handles and variants.
- spec/generator/src/shrink.rs: `Case` (one engine step and the store before it, every earlier event folded in), its candidates (the queue and bookkeeping rows, collections and sources the run names nothing of, items by halves then one at a time, generated mail text cut to one letter with sizes and push keys restated), `candidates` and `Kept` for a model, `shrink` and `signature`.
- spec/generator/src/vector.rs: a `Case` written as a vectors/sync case at format 11, each body a fixture `generated-<object name>.<ext>`; a head, a run cut after a push or a mutation refused.
- spec/generator/src/lib.rs: `REVISION`, 1, which a recorded campaign names.
- tests/generated/campaign.rs: the runner, an ignored test over tests/generated/campaigns.json (campaign-1 and campaign-2, distinct seeds, every profile in each), writing jobs.jsonl, differences.jsonl and summary.txt outside the tree. tests/generated/shrink.rs: the shrinker's driver, a failure's key to a sync vector or an io-pimdir test.
- tests/generated: each suite answers over a model or a history as well as a seed (`reader::disagreements_of`, `overlay::differences_of`, `drain::differences_with`, `index::differences_of`, `search::differences_of`, `crash::owner_cuts`, `index_cuts`, `engine_cuts`, `replay::replay` with step indices, `replay_case`, `replay::asked`, `scale::differences`); `scale::sender` is shared with the campaign's drain.

## Sizing

One release run of every profile on 32 cores, every worker busy: a structure seed about 165 core-seconds, histories 735, quirks 330, count at 5,000 items 40, bytes at full size 80. Each recorded campaign is about 100,000 core-seconds, about an hour on 32 cores. A count store grows worse than linearly (2,000 items 11 s, 5,000 items 36 s, 20,000 items past 20 minutes).

## The shrinking proof

src/placement.rs `PimdirFlags::merge` planted in a scratch copy to keep every local flag (`local.iter()` for `local ∩ remote`): campaign-1's histories profile over two seeds failed six replay jobs; `PIMDIR_SHRINK=histories:1101:replay:mail:5` shrank the first (the queue, the bookkeeping rows and two items dropped, the text of every message cut) into vectors/sync/233 with six fixtures. pimdir's checks/vectors.py passes it (233 sync cases); vectors_sync fails it on the planted copy ("pushes differ") and passes it on this tree. The case is not landed: it pins vector 26's rule again.

## Found on the way (unfixed, by door)

- Spec minor, reads two ways: a DAV listing carrying another body at the revision the base holds (an ETag that does not move on a server edit): io-pimdir adopts the carried body and its summary, the model keeps the old one; SYNC §5 decides a pull by the revision alone and says nothing of a carried body at the base revision (quirks `revisions` step 2, every seed).
- Implementation (io-pimdir, data): a listing's meta stating another `Message-ID` for a bound message whose body the store holds is written over that body's summary (`message_id` the server's rewritten one, the link id and the body the old); STORAGE Annex A: a writer holding the body MUST NOT write a server value over the body's (quirks `rewrites` step 4, the append answered with a handle).
- Spec minor, reads two ways: the same listing over a bound message holding no body restates its size from the new meta in io-pimdir; the model keeps the first summary (quirks `rewrites` step 4).
- Generator's or spec minor: a renumbering that hands every member its old handle; the model bumps the generation, io-pimdir writes no `Rekeyed` drop and leaves it, SYNC §8 bumping it with such a drop (quirks `renumbered` step 3, seeds 1201 and 1208).
- Implementation (io-pimdir, no loss): a round of several pages parked by a mass absence leaves `round_checkpoint` `NULL` uncut, the checkpoint its first page carried when cut after a commit and run again (quirks `extremes` step 6, crash suite).
- Test harness: a first round of about 1,000 members checkpoints the log itself, so the crash suite cannot cut it (quirks `extremes` step 3).
- Test harness, known class: the automatic `sender` references an owner MAY record (G21) are accepted by the scale suite and now the campaign's drain, not by the replay nor search (seed 1207 `renumbered` step 5; structure seed 3 in search, left to the lane editing search.rs).
- To triage (count profile, seeds 1 and 2 at 2,000 and 5,000 items, the scale suite once drained): a queued move or copy into an archive holding the identity leaves, in io-pimdir, the archive's placement at `Meta` with no base body and a pending create `dup:<key>#\u{1}<key>` holding the body; the model's drained archive placement holds the body at `Full`, base and shared, and no minted row (club/Archive seq 1145 `garden.1424@example.com` at seed 1).

No spec break among them. Capabilities moved: none (tests and the generator only).
