---
cairn: log
change: an-unread-arrival-landing-a-create-conflicts
date: 2026-10-30
---

# An unread arrival landing a create conflicts

pimdir sync-draft-05 stated the audit's S8 (pimdir log sync-gaps-io-pimdir-found): a mutable arrival carrying no body that lands a pending create is the both-changed case with its incoming body unknown.

## What landed

- src/sync.rs `land`: such a landing leaves the binding conflicted at the listed revision, no `conflict_object`, its base at that revision naming no body; the page reports `Conflicted` after `Created` (`Fresh::Landed { provisional, conflicted }`) and pushes nothing.
- src/upgrade.rs `land`: a mutable fetch carrying no body does the same; one carrying another body than the staged one records it as `conflict_object`, the upgrade knowing no source policy. An equal body lands clean.
- Failing first: vector 97 ("events differ": no `Conflicted`); src/upgrade.rs `an_unread_fetch_landing_a_mutable_create_conflicts` (left Clean), `a_fetched_body_other_than_the_create_conflicts` (left Dirty). Vectors 95, 96, 98, 99 green.
- Capabilities moved: sync ("An unread arrival landing a create conflicts", added), upgrade ("A pending create is landed by its arrival", modified).

## Left open

SYNC §6 says the upgrade's fetched body, when it differs, is then settled by the source's policy as a carried body would have been. The upgrade carries no source policy, and the next sync leaves a binding conflict whose revision is the listed one untouched, so the conflict waits for a person (`Manual`) under every policy. Settling it at the next sync under `PreferRemote` needs the hub to keep the dropped staged body, which its pull detection (a base revision move) does not see here.
