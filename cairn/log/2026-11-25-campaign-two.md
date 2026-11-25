---
cairn: log
change: campaign-two
landed: 2026-11-25
---

# Campaign two

Campaign-2 (tests/generated/campaigns.json) ran once on 52ea926: 4,180 jobs in 9,714 s on 32 workers, one seed failing, structure 2047, in its drain and search suites (2 and 19 differences), none in the engine, none in the standard. Campaign-1 having broken the standard (Q1), campaign-2 is the first of the two in a row with no spec break that the freeze plan's stop rule (pimdir cairn/changes/the-freeze-plan, D5) asks; campaign-3 is recorded for the second.

## The finding, the harness's

**A `sender` reference outlives the address it was recorded by.** Shrunk (`PIMDIR_SHRINK=structure:2047:drain`) to one card and two queue rows: an add of a mail from `erin.petit@example.com` while the card holds that address, then an update of the card to another address. io-pimdir records the reference when the add drains and keeps it through the update, as STORAGE §14.2 states (a reference is recorded once and never recomputed; an address removed removes nothing). The harness accepted an automatic reference only when the rule stated it over the model once drained, where the card no longer holds the address.

tests/generated/scale.rs `record` keeps the references a store may hold across the states its owner writes: each the rule states over a state joins, and one goes with the last row of either end. `drained_senders` walks a drain row by row, recording after each add or update. The drain suite (`drain::differences_of`, the campaign's and the shrinker's entry alike), the search suite over its writes and the replay over its steps accept a reference recorded in any state they pass; a mail must still hold every reference the rule states over the current model. The search oracle's corpus takes the same set.

tests/generated/drain.rs `a_sender_reference_outlives_the_address_it_was_recorded_by` pins the shrunk case: two differences unaccepted by the old harness, none by the new. Campaign-2's structure profile run again on the new harness (448 jobs, 979 s on 32 workers) fails no seed.

## Left open

The shrinker still writes a drain test asserting no difference at all, an accepted one included; the pinned test filters the known ones by hand. Its signature issue (cairn/log/2026-11-25-campaign-one-rechecked.md) stands.
