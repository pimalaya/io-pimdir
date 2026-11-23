---
cairn: change
id: an-unread-arrival-landing-a-create-conflicts
status: landed
created: 2026-10-30
---

# An unread arrival landing a create conflicts

## Why

pimdir sync-draft-05 now states what this crate's audit (S8, closed as unproven) asked (its log sync-gaps-io-pimdir-found, 2026-10-30): a mutable arrival carrying no body that lands a pending create is the both-changed case with its incoming body unknown (SYNC §6). This crate lands it with `base_object` the staged body at the listed revision, claiming the server holds that body, which nothing read: an edit another client made before the listing is never fetched, and a later local edit pushes over it gated on that revision. Vector 97 fails ("events differ": no `Conflicted`).

## What

- A page landing a pending create under a member listed at a revision with no body, the create holding a body, records the binding as `Manual` records a divergence, whatever the policy: conflicted at the listed revision, no `conflict_object`, the base at that revision naming no body; it reports `Conflicted` beside `Created` and pushes nothing.
- An upgrade landing a pending create does the same when its fetch carries no body (a `Meta` tier, or a `Full` fetch answering none), and records a fetched body differing from the staged one as the divergence's `conflict_object`, the upgrade knowing no source policy.
- The upgrade fetching the body of such a binding clears it on an equal body (vector 98, already green) and records any other in `conflict_object`.
- Vectors 95, 96, 98 and 99 stay green.
