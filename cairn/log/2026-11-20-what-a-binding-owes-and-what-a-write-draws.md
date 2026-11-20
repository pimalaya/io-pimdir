---
cairn: log
change: the-crate-follows-draft-15
landed: 2026-11-20
---

# What a binding owes and what a write draws

The crate follows pimdir's sync-draft-15 rules of cairn/log/2026-11-20-what-a-binding-owes-and-what-a-write-draws.md there, and fixes G12 and G13 of tests/generated/replay.rs.

## What landed

- src/rekey.rs: `PimdirRekeyReport::events`, `Added` for each member no old placement resolves to, named in the rekey's batch, reported once it committed (SYNC §5, §8); the report is no longer `Copy`. tests/vectors_sync.rs compares a rekey's events (vector 190).
- src/hub.rs `PimdirHub::created_placement` (G13): an item under a `hash:` key is offered to no other source (SYNC §9, vector 191).
- src/client/write.rs (G12): a kept body whose summary the hub does not hold takes the summary and the sort key its own bytes give, never the winning item's sort key (SYNC §9, STORAGE Annex A, vector 117).
- src/load.rs, src/hub.rs, src/client/write.rs, src/sync.rs (G14): `PimdirLoaded::folded`, answered by `PimdirHub::folded`, names the placements whose item holds a body another source folded in since the binding last agreed with it. Their both-changed case is pulled whatever the source's policy, `KeepLocal` aside, and the write settles the divergence by the collection's policy (SYNC §5, §9, vectors 192, 193).
- src/hub.rs `absorb_upsert`: an upsert of a mutable kind whose body the item did not take, the item conflicted or keeping its own, leaves the item's summary and sort key as they were: the item kept its body under the incoming one's sort key (vectors 183, 193 stating it).
- Vectors 189 (a carried body equal to the edit adopted as the lost push) and 194 (a write's seqs, named members before kept bodies) pass already.

## Proof

On 6678962: 117, 190 (the runner reading rekey events), 191, 192 and 193 fail; all pass after.

## Capabilities moved

rekey (a new member reported `Added`), hub (a `hash:` item offered to nobody; `folded`), sync (a both-changed binding owing another source's change is the collection's divergence; a kept body's own sort key).
