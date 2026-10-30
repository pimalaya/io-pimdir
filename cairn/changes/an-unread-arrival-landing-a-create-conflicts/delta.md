---
cairn: delta
change: an-unread-arrival-landing-a-create-conflicts
---

## ADDED Requirements

### Requirement: An unread arrival landing a create conflicts
A mutable member listed at a revision with no body that lands a pending create holding a body SHALL leave the binding conflicted at the listed revision with no `conflict_object`, its base at that revision naming no body, whatever the source's policy (SYNC §6): nothing has read what the server holds at that revision. The page SHALL report `Conflicted` beside the landing's `Created` and push nothing for it. An upgrade landing a create from a fetch carrying no body SHALL do the same; one carrying a body other than the staged one SHALL record it as the divergence's `conflict_object`, the base at the fetched revision naming no body. The upgrade fetching the body of such a binding clears the conflict when it equals the staged one, the add whose record was lost having landed.

#### Scenario: The vectors
- vectors/sync 97 (the landing under `prefer-local`) and 98 (the upgrade fetching the staged body clears), tests/vectors_sync.rs
- src/upgrade.rs `an_unread_fetch_landing_a_mutable_create_conflicts`, `a_fetched_body_other_than_the_create_conflicts`

## MODIFIED Requirements

### Requirement: A pending create is landed by its arrival
A page or a fetch resolving a member to a hint a pending create of the same source holds in the collection, a `Created` placement with no base under a provisional handle, SHALL land that create rather than mint a second copy (pimdir SYNC §6): the hint arrived by a relocation the source's remove made, by an accepted add whose record was lost, or by another client, and it is the create delivered. Only a hint a based binding holds is minted.

Landing is a `Superseded` drop of the provisional handle, then the create upserted under the fetched handle in the same batch, with a base of the flags the source reported, the fetched revision, and the fetched body at `Full` or else, for an immutable kind, the create's own body. The flags, body, summary and sort key staged on the create stay, so an edit made on it still pushes: the status is `Clean` when they equal the base and `Dirty` otherwise, and the origin goes, there being nothing left to copy from. A create holding no body adopts a fetched one as its own. A mutable fetch carrying no body, or another body than the staged one, leaves the binding conflicted at the fetched revision, its base naming no body, the fetched body its `conflict_object` ("An unread arrival landing a create conflicts" in [sync](sync.md)).

The `Links` load an upgrade makes for a fresh hint asks for the hint and its minted form, and a store answers with the placements the source binds (SYNC §10), a pending create being one, so the holder to land is in the batch whether or not the requested handles named it.

#### Scenario: A relocated member arrives
- GIVEN a pending create under `tmp-1` holding `m1`
- WHEN a page lists `7` with the hint `m1`, or a fetch of an unnamed `7` resolves to it
- THEN `tmp-1` is dropped `Superseded` and the create is written under `7`, based on what the fetch reported, its body kept

#### Scenario: A held hint is still minted
- GIVEN a based binding holding `m1` under `u1`
- WHEN a fetch of `7` resolves to `m1`
- THEN `7` is linked under `dup:m1#7` and nothing is superseded
