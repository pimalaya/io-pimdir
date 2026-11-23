---
cairn: change
id: engine-conformance
status: landed
created: 2026-09-04
---

# Engine conformance with the revised SYNC part

## Why

The standard's SYNC part was revised on 2026-09-04 (pimdir/cairn/log/2026-09-04-*.md) and fourteen vectors joined the ten the engine reproduced. Run against the new text the I/O-free core diverges in nine places: the change key digests a flag count as eight little-endian bytes where SYNC §4 fixes decimal ASCII; a tombstone's destination was carried on a column no store persists, where §3 now derives it from the pending create the same source holds elsewhere; a fetched hint held by a pending create was minted into a second copy, where §6 lands the create; an accepted content push rebased the placement read before the flag merge, losing the pulled flag §5 says it must keep; a rekey wrote a base claiming the fetched revision it never reconciled, where §8 carries a pull or a conflict; a re-listed probe with unchanged flags was rewritten and reported pulled every run; a rejected result for a handle nobody pushed counted; a tombstone upsert dropped the flags §9 says ride along; and a source whose sibling absorbed a content pull projected dirty on every load. Beside them, the vector harness compared a push on its kind and handle alone, and several property tests wrapped a verb in `let _ =` so a refused write could not fail them.

## What

Every divergence above, in the engine and its suites alone: the key derivation, the destination on the tombstone's origin as the store derives it, the landing of a pending create by its arrival, one pending map the flag axis updates, the rekey's remote-edit rule and a chunked `Meta` fetch, the probe and report rules, the hub's tombstone flags and body-less projection, a `PimdirDeletePolicy::Auto` default the std client resolves from the binding count, the harness comparing every field a push carries and running a `mutate` verb, the property tests made able to fail, and the coroutine states named in the present tense with a completed coroutine refusing every resume. The oversized test modules move beside their modules, and the join out of the sync.
