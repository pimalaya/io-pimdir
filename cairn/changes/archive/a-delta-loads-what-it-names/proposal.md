---
cairn: change
id: a-delta-loads-what-it-names
status: landed
created: 2026-10-18
---

# A delta loads what it names, not the whole collection

## Why

The 2026-10-11 performance audit (p1p2-report #2) found every delta page and every round's last page loading the whole collection: src/sync.rs:589, `PageKind::Delta | PageKind::Last => PimdirLoadSelection::All`, re-verified on 2026-10-18 at 3b4bed8. A step of 50 messages into a 425k-message account loads 92.8 MiB of placements and spends 22.5 s of its 28 s in loads at 1M; at 100k it is 1.3 to 1.7 s. An incremental pass therefore scales with the mailbox, not with what changed. android cairn/changes/store-performance lists it as library proposal L3.

The whole collection is loaded because a delta joins every local placement against what the source listed: a placement the source did not list may still owe a push (a flag change, a local edit, a tombstone, a pending create, a `Created` projection of another source's item), and naming a member needs the link ids the collection holds (SYNC §6). The last page also reads what the round did not stamp, which `PimdirLoaded::unstamped` already answers by statement.

## What

- A delta page and a round's last page load the `Handles` they list and find vanished, the `Links` of the hints they carry (as a page before the last already does), and the placements that owe a push, never `All`.
- The last page takes its absent members from `unstamped` (`list_unstamped_bindings`), which it already asks for, rather than from a whole hub.
- **What owes a push** is the open design point, since it is the one predicate not indexed today. Candidates:
  - a new load scope, `Pending`, answered from the change feed: the collection's items stamped above the stamp the source's last landed delta saw (`items_by_changed`), plus the placements left pending by an earlier run (a rejected push, a held create), which the feed does not restamp and would need a marker of their own;
  - or a partial index over bindings that differ from their base, kept by the writer.
  Either moves pimdir SYNC §10 (the scopes, and which steps "reason about what is missing") and STORAGE §14 (the statement answering it), so the pimdir side lands with it, guarded by a sync vector.
- Behaviour does not change: the same pushes, events and writes as a whole-collection load, which the engine suites and every vector under vectors/sync pin.

## Not in scope

The rekey and `PimdirOpen` keep `All`: both reason about the whole collection by definition.
