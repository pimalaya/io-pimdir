---
cairn: log
change: the-engine-follows-sync-draft-05
date: 2026-10-30
---

# The engine follows sync-draft-05

The engine side of pimdir sync-draft-05 not covered by the merge, the seam or completeness changes: vectors 79, 87, 88, 92, 93 and 94, events tied to commit, and the audit's S11, S13 and S15. Every rule has a vector or a test that failed first.

## What landed

- src/sync.rs: an add accepted with no handle stays pending, unreported (vector 92, S11); a page lands an offered or pending create only when its mail summary names the same message (vector 93); every body a chunk uploads is verified through `PimdirYield::WantsVerify` first, a failing one withheld and counted in `PimdirSyncReport::unreadable` (vector 94); events wait in the coroutine until their write is answered; a carried body equal to the local edit settles it (S13); the flag axis with no base merges the union (S15).
- src/client/write.rs: `PimdirSourceStore::verify`, streaming each blob through the store's hasher; the write keys apart a member binding another source's item whose summary names another message, since a page before the last and an upgrade name members against this source's holders alone (SYNC §10); `item_from_row` no longer special-cases level 0, which `repair_levels` repairs before an owner reads.
- src/mutate.rs: `PimdirMutation::Edit { if_match }`, `PimdirMutateError::Replaced` (vector 87); an unreadable body fails the harness's mutation as the drain's read does (vector 88).
- src/placement.rs: `PimdirLinkId::stated`; src/change.rs, src/remote.rs: seam docs.
- tests/vectors_sync.rs: `failed`, `parked`, `accept_absences`, `blob` false, `corrupt`, `if_match`, events of every verb, rows compared on the columns each expected row names (the union of every row's columns read an unnamed `retained` as a difference), every failing vector named at once.
- Failing first: vectors 92 ("events differ"), 93 and 94 ("pushes differ"); tests/another_message.rs ("left: basic-1@example.org, right: dup:basic-1@example.org#77"); src/sync.rs `an_event_waits_for_the_write_recording_it` ("reported before its write: [Added]", run against the old `emit`), `a_carried_body_equal_to_the_edit_settles_it` ("left: 1, right: 0" conflicts), `no_base_merges_the_union_of_both_sets` (no push).
- Capabilities moved: sync (added "An add accepted without a handle waits for its page", "Another message under a held Message-ID is minted apart", "A body is verified before it is pushed", "An event follows its commit", "A carried body equal to the edit settles it", "The flag axis with no base merges the union"), seam ("A stated key is named on a server without its escape"), mutate (the `Edit` bullet, the report), store and upgrade (level 0).

## Vector errata

Two vectors contradict the prose of SYNC.md and are reported to pimdir; tests/vectors_sync.rs `erratum` corrects each and fails once the vector is fixed upstream:

- 76: jmap holds no coverage, so its run opens its first round, whose page stamps the bindings it lists (SYNC §5, rounds); the vector states `round: null` for binding Ma, where vector 75 stamps imap's 12 with `1` on the same terms.
- 93: source b lacks basic-1 and the store holds its body, so b is offered it and the round's last page pushes its `Add` once member 77 is minted apart (SYNC §9 propagation, §5 creates wait for the page that lands them, which this page did not); the vector states no push.

## Migration for the frontends and connectors

- `PimdirSourceStore::mutate` answers `PimdirMutateReport`; `PimdirMutation::Edit` names `if_match`.
- A runner of its own handles `PimdirYield::WantsVerify` through `service`, or answers `PimdirArg::Verified`.
- A connector names an `Add`'s or a `Remove`'s `link_id` to its server as `link_id.stated()`.
