---
cairn: delta
change: the-agreed-body-stays-pinned
---

## MODIFIED Requirements

### Requirement: A binding records what it last agreed with the hub on
`PimdirBinding` SHALL carry `shared_object`, the shared body this source last agreed with, and the hub SHALL treat it as the base of the cross-source merge. A live upsert carrying a body SHALL move it to the body the item settles on, adopted or kept; an upsert carrying none (a pull at `Meta`, which agreed to no body) and one leaving the item conflicted, a divergence included, the conflict being settled against it, SHALL NOT move it (pimdir SYNC §9, vectors/sync/28, 33, 80 to 84); a `Tombstone` adopts no content and leaves it where it stood. Until a source has folded once it is `None`, and the sync base stands in for it. The store pins it as it pins the base (STORAGE §5).

This is a second base, for the second axis, and the two cannot be one field. `base` is what the source last agreed with its own remote and only a sync moves it, so it stays behind while an unpushed edit waits, which is what keeps the push derivable. The shared axis needs the opposite: a body this source folded in is agreed with, pushed or not.

#### Scenario: A binding that has never folded
- GIVEN a bound source whose binding carries no `shared_object`
- WHEN an upsert is absorbed for it
- THEN the cross-source comparison is made against its sync base

#### Scenario: A body-less pull keeps the agreed body
- GIVEN two sources agreeing on `c1`, and one pulling its server's edit with no body
- WHEN the pull is absorbed
- THEN the item holds no body at `Meta` and the pulling binding still agrees `c1` (vectors/sync/80)

### Requirement: The hub resolves cross-source content conflicts by policy
`PimdirHubItem` SHALL carry a `conflicted` flag and a `conflict_object`, and `PimdirHub` a `PimdirHubConflict` policy (`Manual`, `PreferIncoming`, `PreferExisting`; default `Manual`). On an upsert, three facts SHALL be read before it lands (pimdir SYNC §9): the source changed when the incoming body differs from the base its binding held, or it leaves a conflicted binding; the item changed when the shared body differs from `shared_object`, the sync base standing in; the two disagree when the bodies differ. An item holding no body has changed for a binding that agreed one, and an upsert carrying no body over an item holding one disagrees with it. All three is a divergence: `Manual` SHALL flag the item and record the incoming body in `conflict_object`, keeping the shared body; `PreferIncoming` SHALL adopt the incoming body; `PreferExisting` SHALL keep the shared body. Only the source having changed is a fast-forward, adopting the incoming body.

Two rules keep every body (vectors/sync/81 to 84):

- An upsert carrying a body over an item holding none, from a binding whose base names no body, fills the item without measuring a divergence, an item conflict standing staying as it is.
- A divergence one of whose bodies nobody has read (the upsert carries none, or the item holds none) SHALL be recorded as `Manual` records one, whatever the policy: the shared body stays, the item is flagged, and its `conflict_object` is the incoming body or waits for the upgrade.

A body a policy does not adopt SHALL be kept (vectors/sync/14, 85, 86): the shared body `PreferIncoming` replaced, the incoming one `PreferExisting` refused, and the local edit a source's `PreferRemote` pull dropped, which reaches the hub as a fast-forward whose base revision moved, replacing a body the binding's base did not hold. The hub records it in `PimdirHub::overruled`, and the write keeps it as a retained item keyed by the identity minted over the settling binding's handle (minted again over a held key), with a `seq` of its own, deleted and retained by the settling source, its body pinned, answering it in `PimdirWritten::overruled`. The summary and sort key of an item that kept its body over the upsert's stay its own.

The three facts SHALL be read against the base the binding held, never the base the upsert carries. Whether the content is mutable SHALL be judged across every binding of the item and the placement's own base, any revision meaning mutable. An immutable kind never diverges: a based binding whose base holds a body while the item holds another SHALL adopt the item's body into its base.

Under `Manual` every binding of a conflicted item SHALL project `Conflict` carrying no `conflict_revision` (pimdir SYNC §3), and `absorb` SHALL clear `conflicted` and `conflict_object` when an upsert settles the item: a live upsert adopted as the shared body, a policy settling a divergence, or a tombstone. A divergence the write records is answered in `PimdirWritten::conflicted`.

A source SHALL NOT diverge from itself: its own body folded into the hub and not yet pushed leaves its sync base behind the shared body, and only `shared_object` tells that gap from another source folding in. An upsert leaving a conflicted binding SHALL count as the source having changed its body whatever body it carries.

Flags are unaffected (element-wise, never conflicting).

#### Scenario: A divergent edit conflicts and preserves both under Manual
- GIVEN two sources agreeing on a body, then each editing it to a different body
- WHEN both upserts are absorbed under `Manual`
- THEN the item is `conflicted`, the shared body is kept, and the diverging body is recorded

#### Scenario: Every binding projects the item's conflict
- GIVEN an item flagged conflicted under `Manual`
- WHEN any source's placements are projected
- THEN each projects `Conflict` with no revision, and an edit through any of them settles the item for all

#### Scenario: A clean fast-forward adopts the new body
- GIVEN two sources agreeing on a body, then only one editing it
- WHEN that upsert is absorbed
- THEN the hub adopts the new body with no conflict

#### Scenario: A second offline edit is not a divergence
- GIVEN one source whose offline edit the hub adopted, with the push still pending
- WHEN a second offline edit from that source is absorbed
- THEN the hub adopts it, and the item is not conflicted

#### Scenario: A local edit over an emptied item
- GIVEN two sources agreeing on a body, one pulling its server's edit with no body
- WHEN the other's offline edit is absorbed
- THEN the item is flagged with the edit as its diverging body and every binding projects `Conflict`, never `Clean` (src/hub.rs `a_local_edit_over_an_emptied_item_is_recorded`)

#### Scenario: A policy's loser is kept
- GIVEN `prefer-incoming`, and a source settling its own conflict by taking `c2` while another folded `c3`
- WHEN the edit is absorbed
- THEN the item holds `c2`, `c3` is a retained item under `dup:<link>#<handle>`, and the mutation reports `Overruled` (vectors/sync/85)

#### Scenario: A resolving edit becomes the shared body
- GIVEN a conflicted binding whose local body the hub holds as the shared one
- WHEN the merged body is absorbed as an ordinary edit
- THEN it becomes the shared body, so the next run pushes the merge rather than the body it replaced

#### Scenario: A resolution restating the source's sync base
- GIVEN the same binding, resolved by keeping the body both sides forked from
- WHEN that edit is absorbed
- THEN the ancestor becomes the shared body, the source having spoken even though its body is the one it last synced

### Requirement: Headless conflict resolution
`PimdirSyncOptions` SHALL carry a `PimdirConflictPolicy` (`Manual`, `PreferLocal`, `PreferRemote`; default `Manual`) applied when content diverges on both sides of a based placement. `Manual` marks the placement conflicted and waits for the consumer's edit, recording the observed remote revision as `conflict_revision` and the diverging remote body as `conflict_object` beside it. `PreferRemote` pulls the remote, the local edit it drops kept by the write as a retained item and reported `Overruled` (pimdir SYNC §5, §9, vectors/sync/14); a pull the write cannot let replace another source's body is recorded as the item's conflict instead and reported `Conflicted` in place of `ContentChanged` (vectors/sync/83). `PreferLocal` pushes the local body as an `Update` gated on the observed remote revision, and falls back to `Manual` when the source may not push content. Immutable-content backends report no revision and so never reach a content conflict.

#### Scenario: PreferRemote discards the local edit
- GIVEN a based placement edited locally and changed on the remote, `conflict = PreferRemote`
- WHEN the collection is synced
- THEN the remote content is pulled, the local body is a retained item, and the run reports `ContentChanged` and `Overruled`

### Requirement: Events report what the remote changed
A sync's `PimdirSyncEvent`s SHALL report what was pulled (`Added`, `FlagsChanged`, `ContentChanged`, `Vanished`), a divergence (`Conflicted`, the source's own or one its write records on the item, which replaces the `ContentChanged` of the pull the item did not take), an accepted add under its assigned handle or a create a page landed (`Created`), and a body a policy did not adopt (`Overruled { handle, link_id }`, naming the retained item), in order, each once the write recording it committed, and nothing for an accepted flag, content or delete push (pimdir SYNC §5). An upgrade and a mutation report `Overruled` alone, on `PimdirUpgradeReport::events` and `PimdirMutateReport::events`; a rekey reports none.

## ADDED Requirements

### Requirement: An item conflict's late body meets the policy
An item conflict recorded for want of its incoming body SHALL be revisited by the `Full` upgrade of each binding whose base names a revision and no body, and only those: the fetched body lands as that binding's body and base, and the write settles it against the item's by the collection's policy as a carried body would have been, `Manual` recording it in `items.conflict_object`, `PreferIncoming` adopting it and `PreferExisting` keeping the shared body, the conflict cleared and the body not adopted kept and reported `Overruled` (pimdir SYNC §6, §9, vectors/sync/84, 90, 91). A binding whose base holds a body is not refetched for the item's conflict.

### Requirement: A write answers what the merge recorded
`PimdirSourceStore::write` SHALL answer a `PimdirWritten` (`conflicted`, the handles whose upsert the write recorded as a divergence on the item; `overruled`, each body a policy did not adopt with the key of the retained item keeping it), and `service` SHALL answer `PimdirYield::WantsWrite` with `PimdirArg::Written`. Every verb accepts `PimdirArg::Write` as a write recording nothing beyond its ops, for a storage of its own.
